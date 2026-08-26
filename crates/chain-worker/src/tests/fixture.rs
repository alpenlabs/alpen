//! Epoch-building fixtures for checkpoint-sync consistency tests.
//!
//! [`build_epoch`] constructs one OL epoch (epoch 1, built on the genesis
//! epoch 0), runs it through the block-sync STF to capture reference values,
//! and derives the checkpoint payload a checkpoint-sync run would consume.
//! The resulting [`BuiltEpoch`] lets a test compare both reconstruction paths.

#![allow(unreachable_pub, reason = "test fixture module")]

use strata_acct_types::{AccountSerial, BRIDGE_GATEWAY_ACCT_ID, BitcoinAmount, MessageEntry};
use strata_asm_checkpoint_types::{
    CheckpointPayload, CheckpointSidecar, CheckpointTip, OLLog as CheckpointOLLog,
    TerminalHeaderComplement,
};
use strata_asm_common::AsmManifest;
use strata_checkpoint_types::EpochSummary;
use strata_codec::decode_buf_exact;
use strata_identifiers::{
    Buf32, Epoch, EpochCommitment, L1BlockCommitment, OLBlockCommitment, SubjectId,
};
use strata_ol_chain_types_v1::{MAX_SEALING_MANIFEST_COUNT, OLBlockHeaderV1, OLBlockV1, OLLog};
use strata_ol_da_types_v1::OLDaPayloadV1;
use strata_ol_params::OLRuntimeParams;
use strata_ol_state_support_types::{
    DaAccumulatingState, IndexerState, IndexerWrites, MemoryStateBaseLayer, WriteTrackingState,
};
use strata_ol_state_types::IStateAccessor;
use strata_ol_state_types_v1::{IStateBatchApplicable, OLStateV1, WriteBatch};
use strata_ol_stf_v1::{
    BlockComponents, execute_block_batch_predrain,
    test_utils::{
        EPOCH_RUNNER_TERMINAL_L1_HEIGHT as TERMINAL_L1_HEIGHT, InboxMmrTracker, SnarkUpdateBuilder,
        TEST_RECIPIENT_ID, TEST_SNARK_ACCOUNT_ID, epoch_runner_run_block as run_block,
        epoch_runner_run_genesis as run_genesis, epoch_runner_seed_accounts as seed_accounts,
        get_snark_state_expect, make_account_id, make_deposit_manifest_for_account,
        make_empty_manifest, make_genesis_state, make_p2wpkh_bosd_descriptor, make_state_root,
        make_withdrawal_payload, snark_inbox_msg_with_data,
    },
    verify_block,
};
use strata_ol_tx_types_v1::{OLTransactionDataV1, OLTransactionV1, TxProofsV1};

/// An ordered epoch layout with one explicit terminal block.
#[derive(Debug, Default)]
pub struct EpochPlan {
    blocks: Vec<BlockPlan>,
    terminal: TerminalPlan,
}

impl EpochPlan {
    /// Creates a plan with an empty terminal block.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends an ordinary block before the terminal.
    pub fn add_block(mut self, block: BlockPlan) -> Self {
        self.blocks.push(block);
        self
    }

    /// Replaces the explicit epoch terminal block.
    pub fn terminal(mut self, terminal: impl Into<TerminalPlan>) -> Self {
        self.terminal = terminal.into();
        self
    }
}

/// The test inputs carried by one physical position in an [`EpochPlan`].
///
/// Snark updates expand into their required GAM and update blocks; manifests
/// occupy the final block at this position. The terminal is specified by its
/// position in [`EpochPlan`], never inferred from its manifests.
#[derive(Debug, Default)]
pub struct BlockPlan {
    update_effects: Vec<UpdateEffect>,
    manifests: Vec<ManifestPlan>,
}

/// The unique terminal block of an epoch plan.
#[derive(Debug, Default)]
pub struct TerminalPlan(BlockPlan);

impl From<BlockPlan> for TerminalPlan {
    fn from(block: BlockPlan) -> Self {
        Self(block)
    }
}

impl BlockPlan {
    /// Creates an empty block plan.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds snark updates to this block position.
    pub fn set_snark_updates(mut self, effects: Vec<UpdateEffect>) -> Self {
        self.update_effects = effects;
        self
    }

    /// Adds a deposit manifest for the seeded snark account.
    pub fn deposit_manifest(mut self) -> Self {
        self.manifests.push(ManifestPlan::Deposit);
        self
    }

    /// Adds `count` empty manifests with consecutive L1 heights.
    pub fn empty_manifests(mut self, count: u32) -> Self {
        assert!(
            count <= MAX_SEALING_MANIFEST_COUNT as u32,
            "a block plan may contain at most {MAX_SEALING_MANIFEST_COUNT} manifests"
        );
        self.manifests
            .extend((0..count).map(|_| ManifestPlan::Empty));
        self
    }
}

/// One built OL epoch with the reference values for cross-mode comparison.
pub struct BuiltEpoch {
    /// Epoch commitment of the built epoch (epoch 1).
    pub epoch_commitment: EpochCommitment,
    /// Index of the previous epoch (genesis epoch 0).
    pub prev_epoch_idx: Epoch,
    /// Summary of the previous (genesis) epoch.
    pub prev_summary: EpochSummary,
    /// Terminal block commitment of the previous (genesis) epoch.
    pub prev_terminal: OLBlockCommitment,
    /// Toplevel state at the start of the epoch (post-genesis).
    pub pre_epoch_state: OLStateV1,
    /// Number of ASM logs buffered immediately before the epoch terminal executes.
    pub pre_terminal_pending_asm_logs: usize,
    /// ASM manifests of the epoch keyed by their L1 height.
    pub manifests_by_height: Vec<(u32, AsmManifest)>,
    /// Checkpoint payload a checkpoint-sync run consumes to reconstruct.
    pub checkpoint_payload: CheckpointPayload,
    /// Epoch final state root produced by block-sync execution.
    pub block_sync_state_root: Buf32,
    /// Epoch summary produced by block-sync execution.
    pub block_sync_summary: EpochSummary,
    /// Merged indexer writes captured by block-sync execution.
    block_sync_indexer_writes: IndexerWrites,
    /// Logs emitted by block-sync execution, in emission order across the epoch.
    block_sync_logs: Vec<OLLog>,
}

impl BuiltEpoch {
    /// Returns the indexer writes captured by block-sync execution.
    pub fn block_sync_indexer_writes(&self) -> &IndexerWrites {
        &self.block_sync_indexer_writes
    }

    /// Returns the logs emitted by block-sync execution, in emission order.
    pub fn block_sync_logs(&self) -> &[OLLog] {
        &self.block_sync_logs
    }
}

/// Builds one OL epoch with the given physical plan and the reference values needed to
/// compare block-sync against checkpoint-sync reconstruction.
pub fn build_epoch(plan: EpochPlan) -> BuiltEpoch {
    let mut state = make_genesis_state();
    let snark_serial = seed_accounts(&mut state);

    let genesis = run_genesis(&mut state);
    let pre_epoch_state = state.clone().into_inner();

    // Build ordinary blocks first. Manifests may appear in any block; the
    // explicit terminal below is what applies their buffered L1 logs.
    let mut blocks: Vec<OLBlockV1> = Vec::new();
    let mut prev = genesis.header().clone();
    let mut manifests_by_height = Vec::new();
    let pre_terminal_pending_asm_logs = {
        let mut executor = PlannedBlockExecutor {
            state: &mut state,
            blocks: &mut blocks,
            snark_serial,
            inbox_tracker: InboxMmrTracker::new(),
            next_manifest_height: TERMINAL_L1_HEIGHT,
            manifests_by_height: &mut manifests_by_height,
        };
        for block in plan.blocks {
            prev = executor.execute(&prev, block, false).0;
        }
        executor
            .execute(&prev, plan.terminal.0, true)
            .1
            .expect("terminal execution captures buffered logs")
    };

    let terminal_block = blocks.last().expect("epoch has a terminal block").clone();
    let terminal_header = terminal_block.header().clone();

    // Run the epoch through the block-sync STF to capture reference values.
    let pre_epoch_layer = MemoryStateBaseLayer::new(pre_epoch_state.clone());
    let BlockSyncResult {
        state: block_sync_state,
        state_root: block_sync_state_root,
        indexer_writes: block_sync_indexer_writes,
        logs: block_sync_logs,
    } = run_block_sync(&pre_epoch_layer, &blocks, genesis.header());

    // Genesis commitment / summary for epoch 0.
    let genesis_commitment =
        OLBlockCommitment::new(genesis.header().slot(), genesis.header().compute_blkid());
    let genesis_epoch_state = pre_epoch_state.epoch_state();
    let genesis_l1 = L1BlockCommitment::new(
        genesis_epoch_state.last_l1_height(),
        *genesis_epoch_state.last_l1_blkid(),
    );
    let prev_summary = EpochSummary::new(
        0,
        genesis_commitment,
        OLBlockCommitment::null(),
        genesis_l1,
        *genesis.header().state_root(),
    );

    // Epoch 1 commitment from the terminal block.
    let terminal_commitment =
        OLBlockCommitment::new(terminal_header.slot(), terminal_header.compute_blkid());
    let epoch_commitment = EpochCommitment::new(
        terminal_header.epoch(),
        terminal_header.slot(),
        *terminal_commitment.blkid(),
    );

    // Full-sync epoch summary, sourced the way `build_epoch_summary` does.
    let post_epoch_state = &block_sync_state;
    let post_epoch_l1 = L1BlockCommitment::new(
        post_epoch_state.epoch_state().last_l1_height(),
        *post_epoch_state.epoch_state().last_l1_blkid(),
    );
    let block_sync_summary = EpochSummary::new(
        terminal_header.epoch(),
        terminal_commitment,
        genesis_commitment,
        post_epoch_l1,
        block_sync_state_root,
    );

    // DA blob and per-update OL logs the checkpoint payload carries.
    let (da_blob, ol_logs) = rebuild_da_and_logs(&pre_epoch_layer, &blocks, genesis.header());

    let tip_l1_height = post_epoch_l1.height();
    let checkpoint_payload = assemble_checkpoint_payload(
        da_blob,
        ol_logs,
        &terminal_header,
        terminal_commitment,
        tip_l1_height,
    );

    BuiltEpoch {
        epoch_commitment,
        prev_epoch_idx: 0,
        prev_summary,
        prev_terminal: genesis_commitment,
        pre_epoch_state,
        pre_terminal_pending_asm_logs,
        manifests_by_height,
        checkpoint_payload,
        block_sync_state_root,
        block_sync_summary,
        block_sync_indexer_writes,
        block_sync_logs,
    }
}

/// A manifest included in a [`BlockPlan`].
#[derive(Debug)]
enum ManifestPlan {
    Deposit,
    Empty,
}

/// Materializes a manifest plan at its assigned L1 height.
fn make_feature_manifest(
    feature: ManifestPlan,
    height: u32,
    snark_serial: AccountSerial,
) -> AsmManifest {
    match feature {
        ManifestPlan::Deposit => make_deposit_manifest_for_account(
            height,
            0,
            snark_serial,
            SubjectId::from([42u8; 32]),
            BitcoinAmount::try_from(150_000_000)
                .expect("amount must not exceed the Bitcoin money supply"),
        ),
        ManifestPlan::Empty => make_empty_manifest(height, height as u8),
    }
}

/// Mutable context used while materializing an [`EpochPlan`].
struct PlannedBlockExecutor<'a> {
    state: &'a mut MemoryStateBaseLayer,
    blocks: &'a mut Vec<OLBlockV1>,
    snark_serial: AccountSerial,
    inbox_tracker: InboxMmrTracker,
    next_manifest_height: u32,
    manifests_by_height: &'a mut Vec<(u32, AsmManifest)>,
}

impl PlannedBlockExecutor<'_> {
    /// Executes one planned block position and returns its header and, for a
    /// terminal, the number of logs buffered before the terminal drains them.
    fn execute(
        &mut self,
        parent: &OLBlockHeaderV1,
        block: BlockPlan,
        is_terminal: bool,
    ) -> (OLBlockHeaderV1, Option<usize>) {
        let mut prev = parent.clone();
        if !block.update_effects.is_empty() {
            prev = run_snark_update_blocks(
                self.state,
                self.blocks,
                &prev,
                &block.update_effects,
                &mut self.inbox_tracker,
            );
        }

        let manifests: Vec<_> = block
            .manifests
            .into_iter()
            .map(|feature| {
                let height = self.next_manifest_height;
                self.next_manifest_height += 1;
                let manifest = make_feature_manifest(feature, height, self.snark_serial);
                self.manifests_by_height.push((height, manifest.clone()));
                manifest
            })
            .collect();
        let pending = is_terminal.then(|| self.state.pending_asm_logs_len());
        let components = if manifests.is_empty() {
            BlockComponents::new_empty()
        } else {
            BlockComponents::new_manifests(manifests)
        };
        (
            run_block(
                self.state,
                self.blocks,
                &prev,
                components.with_terminal(is_terminal),
            ),
            pending,
        )
    }
}

/// Assembles the checkpoint payload from the DA blob, OL logs, and terminal header.
fn assemble_checkpoint_payload(
    da_blob: Vec<u8>,
    ol_logs: Vec<CheckpointOLLog>,
    terminal_header: &OLBlockHeaderV1,
    terminal_commitment: OLBlockCommitment,
    tip_l1_height: u32,
) -> CheckpointPayload {
    let complement = TerminalHeaderComplement::new(
        terminal_header.timestamp(),
        *terminal_header.parent_blkid(),
        *terminal_header.body_root(),
        *terminal_header.logs_root(),
    );
    let sidecar =
        CheckpointSidecar::new(da_blob, ol_logs, complement).expect("build checkpoint sidecar");
    let tip = CheckpointTip::new(terminal_header.epoch(), tip_l1_height, terminal_commitment);
    CheckpointPayload::new(tip, sidecar, Vec::new()).expect("build checkpoint payload")
}

/// Reference values captured from a block-sync run of an epoch.
struct BlockSyncResult {
    state: OLStateV1,
    state_root: Buf32,
    indexer_writes: IndexerWrites,
    logs: Vec<OLLog>,
}

/// Runs the epoch's blocks through the block-sync STF, accumulating the write
/// batch, indexer writes, and emitted logs across all blocks into a single pass.
fn run_block_sync(
    pre_epoch_state: &MemoryStateBaseLayer,
    blocks: &[OLBlockV1],
    genesis_header: &OLBlockHeaderV1,
) -> BlockSyncResult {
    let tracking_state = WriteTrackingState::new_empty(pre_epoch_state);
    let mut indexer_state = IndexerState::new(tracking_state);

    let mut prev_header = genesis_header.clone();
    let mut logs = Vec::new();
    for block in blocks {
        let block_logs = verify_block(
            &mut indexer_state,
            block.header(),
            Some(&prev_header),
            block.body(),
            &OLRuntimeParams::default(),
        )
        .expect("block-sync verify_block");
        logs.extend(block_logs);
        prev_header = block.header().clone();
    }

    let (tracking_state, indexer_writes) = indexer_state.into_parts();
    let write_batch: WriteBatch = tracking_state.into_batch();

    let mut new_state = pre_epoch_state.clone();
    new_state
        .apply_write_batch(write_batch)
        .expect("apply block-sync write batch");
    let state_root = new_state
        .compute_state_root()
        .expect("block-sync state root");

    BlockSyncResult {
        state: new_state.into_inner(),
        state_root,
        indexer_writes,
        logs,
    }
}

/// Rebuilds the epoch DA blob and per-update OL logs via the checkpoint-builder
/// preseal path.
fn rebuild_da_and_logs(
    pre_epoch_state: &MemoryStateBaseLayer,
    blocks: &[OLBlockV1],
    genesis_header: &OLBlockHeaderV1,
) -> (Vec<u8>, Vec<CheckpointOLLog>) {
    let mut da = DaAccumulatingState::new(pre_epoch_state.clone());
    let logs =
        execute_block_batch_predrain(&mut da, blocks, genesis_header, &OLRuntimeParams::default())
            .expect("execute_block_batch_predrain");
    let blob = da
        .take_completed_epoch_da_blob()
        .expect("finalize DA")
        .expect("DA blob");
    let _: OLDaPayloadV1 = decode_buf_exact(&blob).expect("DA blob decodes");
    let ol_logs = logs
        .into_iter()
        .map(|l| CheckpointOLLog::new(l.account_serial(), l.payload().to_vec()))
        .collect();
    (blob, ol_logs)
}

/// The effect a snark update applies, beyond consuming its inbox message.
#[derive(Debug)]
pub enum UpdateEffect {
    /// No transfer or output message; only advances the account cursor.
    None,
    /// Transfer sats to the test recipient account.
    Transfer(u64),
    /// Emit a bridge withdrawal of one denomination to the bridge gateway.
    Withdrawal,
}

/// Runs one GAM + one snark update per effect, returning the last block's header.
///
/// The epoch-scoped `tracker` mirrors every message the live snark inbox has
/// accepted, so proofs remain valid across multiple planned update blocks.
fn run_snark_update_blocks(
    state: &mut MemoryStateBaseLayer,
    blocks: &mut Vec<OLBlockV1>,
    genesis_header: &OLBlockHeaderV1,
    effects: &[UpdateEffect],
    tracker: &mut InboxMmrTracker,
) -> OLBlockHeaderV1 {
    let snark_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let msgs: Vec<MessageEntry> = (0..effects.len())
        .map(|i| snark_inbox_msg_with_data(format!("msg-{i}").as_bytes()))
        .collect();

    let mut prev = genesis_header.clone();
    for msg in &msgs {
        let gam = OLTransactionV1::new(
            OLTransactionDataV1::from_gam_bytes(snark_id, msg.payload().data().to_vec())
                .expect("gam payload"),
            TxProofsV1::new_empty(),
        );
        prev = run_block(
            state,
            blocks,
            &prev,
            BlockComponents::new_txs_from_ol_transactions(vec![gam]),
        );
    }

    let first_message_index = tracker.num_entries() as usize;
    for msg in &msgs {
        tracker.add_message(msg);
    }

    for (idx, effect) in effects.iter().enumerate() {
        let (_, snark_state) = get_snark_state_expect(state, snark_id);
        let mut builder = SnarkUpdateBuilder::from_snark_state(snark_state.clone())
            .with_processed_msgs(vec![msgs[idx].clone()])
            .with_inbox_proofs(vec![tracker.proof_for(first_message_index + idx)]);
        builder = match effect {
            UpdateEffect::None => builder,
            UpdateEffect::Transfer(amount) => {
                builder.with_transfer(make_account_id(TEST_RECIPIENT_ID), *amount)
            }
            UpdateEffect::Withdrawal => builder.with_output_message(
                BRIDGE_GATEWAY_ACCT_ID,
                100_000_000,
                make_withdrawal_payload(make_p2wpkh_bosd_descriptor(0x14)),
            ),
        };
        let update = builder.build(snark_id, make_state_root(idx as u8 + 2), vec![0u8; 32]);
        prev = run_block(
            state,
            blocks,
            &prev,
            BlockComponents::new_txs_from_ol_transactions(vec![update]),
        );
    }

    prev
}
