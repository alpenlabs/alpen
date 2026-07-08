//! Epoch-building fixtures for checkpoint-sync consistency tests.
//!
//! [`build_epoch`] constructs one OL epoch (epoch 1, built on the genesis
//! epoch 0), runs it through the block-sync STF to capture reference values,
//! and derives the checkpoint payload a checkpoint-sync run would consume.
//! The resulting [`BuiltEpoch`] lets a test compare both reconstruction paths.

#![allow(unreachable_pub, reason = "test fixture module")]

use strata_acct_types::{BRIDGE_GATEWAY_ACCT_ID, BitcoinAmount, MessageEntry};
use strata_asm_common::AsmManifest;
use strata_asm_proto_checkpoint_types::{
    CheckpointPayload, CheckpointSidecar, CheckpointTip, OLLog as CheckpointOLLog,
    TerminalHeaderComplement,
};
use strata_bridge_params::BridgeParams;
use strata_checkpoint_types::EpochSummary;
use strata_codec::decode_buf_exact;
use strata_identifiers::{
    Buf32, Epoch, EpochCommitment, L1BlockCommitment, OLBlockCommitment, SubjectId,
};
use strata_ledger_types::IStateAccessor;
use strata_ol_chain_types::{
    MAX_SEALING_MANIFEST_COUNT, OLBlock, OLBlockHeader, OLLog, OLTransaction, OLTransactionData,
    TxProofs,
};
use strata_ol_da::OLDaPayloadV1;
use strata_ol_state_support_types::{
    DaAccumulatingState, IndexerState, IndexerWrites, MemoryStateBaseLayer, WriteTrackingState,
};
use strata_ol_state_types::{IStateBatchApplicable, OLAccountState, OLState, WriteBatch};
use strata_ol_stf::{
    BlockComponents, execute_block_batch_predrain,
    test_utils::{
        EPOCH_RUNNER_TERMINAL_L1_HEIGHT as TERMINAL_L1_HEIGHT, InboxMmrTracker, SnarkUpdateBuilder,
        TEST_RECIPIENT_ID, TEST_SNARK_ACCOUNT_ID, epoch_runner_run_block as run_block,
        epoch_runner_run_genesis as run_genesis, epoch_runner_run_terminal as run_terminal,
        epoch_runner_seed_accounts as seed_accounts, get_snark_state_expect, make_account_id,
        make_deposit_manifest_for_account, make_empty_manifest, make_genesis_state,
        make_p2wpkh_bosd_descriptor, make_state_root, make_withdrawal_payload,
        snark_inbox_msg_with_data,
    },
    verify_block,
};

/// The shape of the epoch [`build_epoch`] constructs.
#[derive(Debug, Clone, Copy)]
pub enum EpochShape {
    /// Empty filler blocks plus a terminal carrying a deposit manifest. No
    /// snark updates: covers the path where `ol_logs` is empty.
    DepositManifestOnly,

    /// Empty filler blocks plus an empty terminal. Covers a sealed epoch with
    /// no new L1 manifests.
    NoNewManifests,

    /// Multiple OL txs, plus a terminal deposit manifest. Exercises per-update record
    /// reconstruction from `ol_logs` (N > 1) alongside deposit-manifest indexing.
    SnarkMultiUpdateAndDeposit,

    /// ASM manifests are carried by non-terminal blocks; the terminal block has no
    /// manifests.
    ManifestsInNonTerminalBlocks,

    /// More ASM manifests than the epoch manifest cap allows.
    ManifestsExceedEpochCap,

    /// A snark update whose output message is a bridge withdrawal, plus an
    /// empty terminal.
    WithdrawalOnly,

    /// A snark update carrying a bridge withdrawal, plus a terminal deposit
    /// manifest.
    WithdrawalAndDeposit,

    /// Everything in one epoch: multiple snark updates, a bridge withdrawal,
    /// and a terminal deposit manifest.
    All,
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
    pub pre_epoch_state: OLState,
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

/// Builds one OL epoch of the given shape and the reference values needed to
/// compare block-sync against checkpoint-sync reconstruction.
pub fn build_epoch(shape: EpochShape) -> BuiltEpoch {
    let mut state = make_genesis_state();
    let snark_serial = seed_accounts(&mut state);

    let genesis = run_genesis(&mut state);
    let pre_epoch_state = state.clone().into_inner();

    // Build the epoch's blocks per shape.
    let mut blocks: Vec<OLBlock> = Vec::new();
    let manifests_by_height = match shape {
        EpochShape::DepositManifestOnly => {
            let mut prev = genesis.header().clone();
            for _ in 0..4 {
                prev = run_block(&mut state, &mut blocks, &prev, BlockComponents::new_empty());
            }
            let manifest = make_deposit_manifest_for_account(
                TERMINAL_L1_HEIGHT,
                0,
                snark_serial,
                SubjectId::from([42u8; 32]),
                BitcoinAmount::from_sat(150_000_000),
            );
            run_terminal(&mut state, &mut blocks, &prev, manifest.clone());
            vec![(TERMINAL_L1_HEIGHT, manifest)]
        }
        EpochShape::NoNewManifests => {
            let mut prev = genesis.header().clone();
            for _ in 0..4 {
                prev = run_block(&mut state, &mut blocks, &prev, BlockComponents::new_empty());
            }
            run_block(
                &mut state,
                &mut blocks,
                &prev,
                BlockComponents::new_empty().as_terminal(),
            );
            Vec::new()
        }
        EpochShape::SnarkMultiUpdateAndDeposit => {
            let prev = run_snark_update_blocks(
                &mut state,
                &mut blocks,
                genesis.header(),
                &[
                    UpdateEffect::Transfer(1_000_000),
                    UpdateEffect::Transfer(1_000_000),
                ],
            );
            let manifest = make_deposit_manifest_for_account(
                TERMINAL_L1_HEIGHT,
                0,
                snark_serial,
                SubjectId::from([42u8; 32]),
                BitcoinAmount::from_sat(150_000_000),
            );
            run_terminal(&mut state, &mut blocks, &prev, manifest.clone());
            vec![(TERMINAL_L1_HEIGHT, manifest)]
        }
        EpochShape::ManifestsInNonTerminalBlocks => {
            let mut prev = genesis.header().clone();
            let manifest_a = make_empty_manifest(TERMINAL_L1_HEIGHT, 2);
            prev = run_block(
                &mut state,
                &mut blocks,
                &prev,
                BlockComponents::new_manifests(vec![manifest_a.clone()]),
            );

            let manifest_b = make_empty_manifest(TERMINAL_L1_HEIGHT + 1, 3);
            prev = run_block(
                &mut state,
                &mut blocks,
                &prev,
                BlockComponents::new_manifests(vec![manifest_b.clone()]),
            );
            run_block(
                &mut state,
                &mut blocks,
                &prev,
                BlockComponents::new_empty().as_terminal(),
            );

            vec![
                (TERMINAL_L1_HEIGHT, manifest_a),
                (TERMINAL_L1_HEIGHT + 1, manifest_b),
            ]
        }
        EpochShape::ManifestsExceedEpochCap => {
            let max_per_block = MAX_SEALING_MANIFEST_COUNT as u32;
            let mut manifests_by_height =
                Vec::with_capacity(MAX_SEALING_MANIFEST_COUNT as usize + 1);

            let first_chunk: Vec<_> = (TERMINAL_L1_HEIGHT..TERMINAL_L1_HEIGHT + max_per_block)
                .map(|height| {
                    let manifest = make_empty_manifest(height, height as u8);
                    manifests_by_height.push((height, manifest.clone()));
                    manifest
                })
                .collect();
            let mut prev = run_block(
                &mut state,
                &mut blocks,
                genesis.header(),
                BlockComponents::new_manifests(first_chunk),
            );

            let overflow_height = TERMINAL_L1_HEIGHT + max_per_block;
            let overflow_manifest = make_empty_manifest(overflow_height, overflow_height as u8);
            manifests_by_height.push((overflow_height, overflow_manifest.clone()));
            prev = run_block(
                &mut state,
                &mut blocks,
                &prev,
                BlockComponents::new_manifests(vec![overflow_manifest]),
            );
            run_block(
                &mut state,
                &mut blocks,
                &prev,
                BlockComponents::new_empty().as_terminal(),
            );

            manifests_by_height
        }
        EpochShape::WithdrawalOnly => {
            let prev = run_snark_update_blocks(
                &mut state,
                &mut blocks,
                genesis.header(),
                &[UpdateEffect::Withdrawal],
            );
            let manifest = make_empty_manifest(TERMINAL_L1_HEIGHT, 0);
            run_terminal(&mut state, &mut blocks, &prev, manifest.clone());
            vec![(TERMINAL_L1_HEIGHT, manifest)]
        }
        EpochShape::WithdrawalAndDeposit => {
            let prev = run_snark_update_blocks(
                &mut state,
                &mut blocks,
                genesis.header(),
                &[UpdateEffect::Withdrawal],
            );
            let manifest = make_deposit_manifest_for_account(
                TERMINAL_L1_HEIGHT,
                0,
                snark_serial,
                SubjectId::from([42u8; 32]),
                BitcoinAmount::from_sat(150_000_000),
            );
            run_terminal(&mut state, &mut blocks, &prev, manifest.clone());
            vec![(TERMINAL_L1_HEIGHT, manifest)]
        }
        EpochShape::All => {
            // Two no-op updates keep the full seeded balance available for the
            // withdrawal, which must equal one denomination.
            let prev = run_snark_update_blocks(
                &mut state,
                &mut blocks,
                genesis.header(),
                &[
                    UpdateEffect::None,
                    UpdateEffect::None,
                    UpdateEffect::Withdrawal,
                ],
            );
            let manifest = make_deposit_manifest_for_account(
                TERMINAL_L1_HEIGHT,
                0,
                snark_serial,
                SubjectId::from([42u8; 32]),
                BitcoinAmount::from_sat(150_000_000),
            );
            run_terminal(&mut state, &mut blocks, &prev, manifest.clone());
            vec![(TERMINAL_L1_HEIGHT, manifest)]
        }
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
        manifests_by_height,
        checkpoint_payload,
        block_sync_state_root,
        block_sync_summary,
        block_sync_indexer_writes,
        block_sync_logs,
    }
}

/// Assembles the checkpoint payload from the DA blob, OL logs, and terminal header.
fn assemble_checkpoint_payload(
    da_blob: Vec<u8>,
    ol_logs: Vec<CheckpointOLLog>,
    terminal_header: &OLBlockHeader,
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
    state: OLState,
    state_root: Buf32,
    indexer_writes: IndexerWrites,
    logs: Vec<OLLog>,
}

/// Runs the epoch's blocks through the block-sync STF, accumulating the write
/// batch, indexer writes, and emitted logs across all blocks into a single pass.
fn run_block_sync(
    pre_epoch_state: &MemoryStateBaseLayer,
    blocks: &[OLBlock],
    genesis_header: &OLBlockHeader,
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
            BridgeParams::default(),
        )
        .expect("block-sync verify_block");
        logs.extend(block_logs);
        prev_header = block.header().clone();
    }

    let (tracking_state, indexer_writes) = indexer_state.into_parts();
    let write_batch: WriteBatch<OLAccountState> = tracking_state.into_batch();

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
    blocks: &[OLBlock],
    genesis_header: &OLBlockHeader,
) -> (Vec<u8>, Vec<CheckpointOLLog>) {
    let mut da = DaAccumulatingState::new(pre_epoch_state.clone());
    let logs =
        execute_block_batch_predrain(&mut da, blocks, genesis_header, BridgeParams::default())
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
enum UpdateEffect {
    /// No transfer or output message; only advances the account cursor.
    None,
    /// Transfer sats to the test recipient account.
    Transfer(u64),
    /// Emit a bridge withdrawal of one denomination to the bridge gateway.
    Withdrawal,
}

/// Runs one GAM + one snark update per effect, returning the last block's header.
///
/// Proofs come from a single MMR tracking every message, so each leaf validates
/// against the final inbox the account sees after all GAMs land.
fn run_snark_update_blocks(
    state: &mut MemoryStateBaseLayer,
    blocks: &mut Vec<OLBlock>,
    genesis_header: &OLBlockHeader,
    effects: &[UpdateEffect],
) -> OLBlockHeader {
    let snark_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let msgs: Vec<MessageEntry> = (0..effects.len())
        .map(|i| snark_inbox_msg_with_data(format!("msg-{i}").as_bytes()))
        .collect();

    let mut prev = genesis_header.clone();
    for msg in &msgs {
        let gam = OLTransaction::new(
            OLTransactionData::from_gam_bytes(snark_id, msg.payload().data().to_vec())
                .expect("gam payload"),
            TxProofs::new_empty(),
        );
        prev = run_block(
            state,
            blocks,
            &prev,
            BlockComponents::new_txs_from_ol_transactions(vec![gam]),
        );
    }

    let mut tracker = InboxMmrTracker::new();
    for msg in &msgs {
        tracker.add_message(msg);
    }

    for (idx, effect) in effects.iter().enumerate() {
        let (_, snark_state) = get_snark_state_expect(state, snark_id);
        let mut builder = SnarkUpdateBuilder::from_snark_state(snark_state.clone())
            .with_processed_msgs(vec![msgs[idx].clone()])
            .with_inbox_proofs(vec![tracker.proof_for(idx)]);
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
