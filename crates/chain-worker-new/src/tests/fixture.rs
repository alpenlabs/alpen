//! Epoch-building fixtures for checkpoint-sync consistency tests.
//!
//! [`build_epoch`] constructs one OL epoch (epoch 1, built on the genesis
//! epoch 0), runs it through the full-sync STF to capture reference values,
//! and derives the checkpoint payload a checkpoint-sync run would consume.
//! The resulting [`BuiltEpoch`] lets a test compare both reconstruction paths.

#![allow(unreachable_pub, reason = "test fixture module")]

use strata_acct_types::{BitcoinAmount, MessageEntry};
use strata_asm_common::AsmManifest;
use strata_asm_proto_checkpoint_types::{
    CheckpointPayload, CheckpointSidecar, CheckpointTip, TerminalHeaderComplement,
};
use strata_checkpoint_types::EpochSummary;
use strata_codec::decode_buf_exact;
use strata_identifiers::{
    AccountSerial, Buf32, Epoch, EpochCommitment, L1BlockCommitment, OLBlockCommitment, SubjectId,
};
use strata_ledger_types::{IStateAccessor, IStateAccessorMut, NewAccountData, NewAccountTypeState};
use strata_ol_chain_types_new::{OLBlock, OLBlockHeader};
use strata_ol_da::OLDaPayloadV1;
use strata_ol_state_support_types::{
    DaAccumulatingState, IndexerState, IndexerWrites, MemoryStateBaseLayer, WriteTrackingState,
};
use strata_ol_state_types::{IStateBatchApplicable, OLAccountState, OLState, WriteBatch};
use strata_ol_stf::{
    BlockComponents, BlockInfo, CompletedBlock, execute_block_batch_preseal,
    test_utils::{
        InboxMmrTracker, SnarkUpdateBuilder, TEST_RECIPIENT_ID, TEST_SNARK_ACCOUNT_ID,
        execute_block, get_snark_state_expect, insert_empty_account, make_account_id,
        make_deposit_manifest_for_account, make_empty_manifest, make_genesis_state, make_state_root,
        snark_inbox_msg, to_ol_block,
    },
    verify_block,
};
use strata_predicate::PredicateKey;

const GENESIS_TIMESTAMP: u64 = 1_000_000;
const SLOT_TIMESTAMP_STEP: u64 = 1_000;

/// L1 height of the epoch's terminal manifest.
///
/// Genesis carries its manifest at height 1; the non-terminal blocks of the
/// epoch under test carry none, so the terminal manifest sits at height 2.
const TERMINAL_L1_HEIGHT: u32 = 2;

/// The shape of the epoch [`build_epoch`] constructs.
#[derive(Debug, Clone, Copy)]
pub enum EpochShape {
    /// Empty filler blocks plus a terminal carrying a deposit manifest.
    DepositManifestOnly,
    /// A GAM-then-snark-update sequence with an empty terminal manifest.
    SnarkUpdate,
    /// A snark update sequence plus a terminal deposit manifest.
    SnarkUpdateAndDeposit,
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
    /// Epoch final state root produced by full-sync execution.
    pub full_sync_state_root: Buf32,
    /// Epoch summary produced by full-sync execution.
    pub full_sync_summary: EpochSummary,
    /// Merged indexer writes captured by full-sync execution.
    full_sync_indexer_writes: IndexerWrites,
}

impl BuiltEpoch {
    /// Returns the indexer writes captured by full-sync execution.
    pub fn full_sync_indexer_writes(&self) -> &IndexerWrites {
        &self.full_sync_indexer_writes
    }
}

/// Builds one OL epoch of the given shape and the reference values needed to
/// compare full-sync against checkpoint-sync reconstruction.
pub fn build_epoch(shape: EpochShape) -> BuiltEpoch {
    let mut state = make_genesis_state();
    let snark_serial = seed_accounts(&mut state);

    let genesis = run_genesis(&mut state);
    let pre_epoch_state = state.clone().into_inner();

    // Build the epoch's blocks per shape.
    let mut blocks: Vec<OLBlock> = Vec::new();
    let terminal_manifest = match shape {
        EpochShape::DepositManifestOnly => {
            let mut prev = genesis.header().clone();
            for _ in 0..4 {
                prev = run_block(&mut state, &mut blocks, &prev, BlockComponents::new_empty());
            }
            let manifest = make_deposit_manifest_for_account(TERMINAL_L1_HEIGHT, 0, snark_serial, SubjectId::from([42u8; 32]), BitcoinAmount::from_sat(150_000_000));
            run_terminal(&mut state, &mut blocks, &prev, manifest.clone());
            manifest
        }
        EpochShape::SnarkUpdate => {
            let prev = run_snark_update_blocks(&mut state, &mut blocks, genesis.header());
            let manifest = make_empty_manifest(TERMINAL_L1_HEIGHT, 0);
            run_terminal(&mut state, &mut blocks, &prev, manifest.clone());
            manifest
        }
        EpochShape::SnarkUpdateAndDeposit => {
            let prev = run_snark_update_blocks(&mut state, &mut blocks, genesis.header());
            let manifest = make_deposit_manifest_for_account(TERMINAL_L1_HEIGHT, 0, snark_serial, SubjectId::from([42u8; 32]), BitcoinAmount::from_sat(150_000_000));
            run_terminal(&mut state, &mut blocks, &prev, manifest.clone());
            manifest
        }
    };

    let terminal_block = blocks.last().expect("epoch has a terminal block").clone();
    let terminal_header = terminal_block.header().clone();

    // Run the epoch through the full-sync STF to capture reference values.
    let pre_epoch_layer = MemoryStateBaseLayer::new(pre_epoch_state.clone());
    let (full_sync_state, full_sync_state_root, full_sync_indexer_writes) =
        run_full_sync(&pre_epoch_layer, &blocks, genesis.header());

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
    let post_epoch_state = &full_sync_state;
    let post_epoch_l1 = L1BlockCommitment::new(
        post_epoch_state.epoch_state().last_l1_height(),
        *post_epoch_state.epoch_state().last_l1_blkid(),
    );
    let full_sync_summary = EpochSummary::new(
        terminal_header.epoch(),
        terminal_commitment,
        genesis_commitment,
        post_epoch_l1,
        full_sync_state_root,
    );

    // DA blob the checkpoint payload carries as its OL state diff.
    let da_blob = rebuild_da_blob(&pre_epoch_layer, &blocks, genesis.header());

    let checkpoint_payload =
        assemble_checkpoint_payload(da_blob, &terminal_header, terminal_commitment);

    BuiltEpoch {
        epoch_commitment,
        prev_epoch_idx: 0,
        prev_summary,
        prev_terminal: genesis_commitment,
        pre_epoch_state,
        manifests_by_height: vec![(TERMINAL_L1_HEIGHT, terminal_manifest)],
        checkpoint_payload,
        full_sync_state_root,
        full_sync_summary,
        full_sync_indexer_writes,
    }
}

/// Assembles the checkpoint payload from the DA blob and terminal header.
fn assemble_checkpoint_payload(
    da_blob: Vec<u8>,
    terminal_header: &OLBlockHeader,
    terminal_commitment: OLBlockCommitment,
) -> CheckpointPayload {
    let complement = TerminalHeaderComplement::new(
        terminal_header.timestamp(),
        *terminal_header.parent_blkid(),
        *terminal_header.body_root(),
        *terminal_header.logs_root(),
    );
    let sidecar =
        CheckpointSidecar::new(da_blob, Vec::new(), complement).expect("build checkpoint sidecar");
    let tip = CheckpointTip::new(
        terminal_header.epoch(),
        TERMINAL_L1_HEIGHT,
        terminal_commitment,
    );
    CheckpointPayload::new(tip, sidecar, Vec::new()).expect("build checkpoint payload")
}

/// Runs the epoch's blocks through the full-sync STF, accumulating the write
/// batch and indexer writes across all blocks into a single pass.
///
/// Returns the post-epoch toplevel state, its state root, and the merged
/// indexer writes.
fn run_full_sync(
    pre_epoch_state: &MemoryStateBaseLayer,
    blocks: &[OLBlock],
    genesis_header: &OLBlockHeader,
) -> (OLState, Buf32, IndexerWrites) {
    let tracking_state = WriteTrackingState::new_empty(pre_epoch_state);
    let mut indexer_state = IndexerState::new(tracking_state);

    let mut prev_header = genesis_header.clone();
    for block in blocks {
        verify_block(
            &mut indexer_state,
            block.header(),
            Some(&prev_header),
            block.body(),
        )
        .expect("full-sync verify_block");
        prev_header = block.header().clone();
    }

    let (tracking_state, indexer_writes) = indexer_state.into_parts();
    let write_batch: WriteBatch<OLAccountState> = tracking_state.into_batch();

    let mut new_state = pre_epoch_state.clone();
    new_state
        .apply_write_batch(write_batch)
        .expect("apply full-sync write batch");
    let state_root = new_state
        .compute_state_root()
        .expect("full-sync state root");

    (new_state.into_inner(), state_root, indexer_writes)
}

/// Rebuilds the epoch DA blob via the checkpoint-builder preseal path.
///
/// Sanity-decodes the blob as an [`OLDaPayloadV1`] so a malformed blob fails
/// here rather than deep inside checkpoint-sync reconstruction.
fn rebuild_da_blob(
    pre_epoch_state: &MemoryStateBaseLayer,
    blocks: &[OLBlock],
    genesis_header: &OLBlockHeader,
) -> Vec<u8> {
    let mut da = DaAccumulatingState::new(pre_epoch_state.clone());
    execute_block_batch_preseal(&mut da, blocks, genesis_header)
        .expect("execute_block_batch_preseal");
    let blob = da
        .take_completed_epoch_da_blob()
        .expect("finalize DA")
        .expect("DA blob");
    let _: OLDaPayloadV1 = decode_buf_exact(&blob).expect("DA blob decodes");
    blob
}

/// Seeds the recipient and snark accounts, returning the snark account serial.
fn seed_accounts(state: &mut MemoryStateBaseLayer) -> AccountSerial {
    insert_empty_account(state, make_account_id(TEST_RECIPIENT_ID));
    state
        .create_new_account(
            make_account_id(TEST_SNARK_ACCOUNT_ID),
            NewAccountData::new(
                BitcoinAmount::from_sat(100_000_000),
                NewAccountTypeState::Snark {
                    update_vk: PredicateKey::always_accept(),
                    initial_state_root: make_state_root(1),
                },
            ),
        )
        .expect("create snark account")
}

/// Executes the genesis (epoch 0 terminal) block.
fn run_genesis(state: &mut MemoryStateBaseLayer) -> CompletedBlock {
    execute_block(
        state,
        &BlockInfo::new_genesis(GENESIS_TIMESTAMP),
        None,
        BlockComponents::new_manifests(vec![make_empty_manifest(1, 0)]),
    )
    .expect("genesis block")
}

/// Executes one block following `parent` with the given components.
fn run_block(
    state: &mut MemoryStateBaseLayer,
    blocks: &mut Vec<OLBlock>,
    parent: &OLBlockHeader,
    components: BlockComponents,
) -> OLBlockHeader {
    let slot = parent.slot() + 1;
    let cb = execute_block(
        state,
        &BlockInfo::new(GENESIS_TIMESTAMP + slot * SLOT_TIMESTAMP_STEP, slot, 1),
        Some(parent),
        components,
    )
    .expect("epoch block");
    blocks.push(to_ol_block(&cb));
    cb.header().clone()
}

/// Executes the terminal block carrying `manifest`, closing the epoch.
fn run_terminal(
    state: &mut MemoryStateBaseLayer,
    blocks: &mut Vec<OLBlock>,
    parent: &OLBlockHeader,
    manifest: AsmManifest,
) -> CompletedBlock {
    let slot = parent.slot() + 1;
    let cb = execute_block(
        state,
        &BlockInfo::new(GENESIS_TIMESTAMP + slot * SLOT_TIMESTAMP_STEP, slot, 1),
        Some(parent),
        BlockComponents::new_manifests(vec![manifest]),
    )
    .expect("terminal block");
    blocks.push(to_ol_block(&cb));
    cb
}

/// Runs the non-terminal blocks of a snark-update epoch: a GAM delivering an
/// inbox message followed by a snark update consuming it.
///
/// Returns the header of the last block, for the terminal to build on.
fn run_snark_update_blocks(
    state: &mut MemoryStateBaseLayer,
    blocks: &mut Vec<OLBlock>,
    genesis_header: &OLBlockHeader,
) -> OLBlockHeader {
    use strata_ol_chain_types_new::{OLTransaction, OLTransactionData, TxProofs};

    let snark_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let inbox_msg = snark_inbox_msg();

    let mut prev = run_block(state, blocks, genesis_header, BlockComponents::new_empty());

    let gam_tx = OLTransaction::new(
        OLTransactionData::from_gam_bytes(snark_id, inbox_msg.payload().data().to_vec())
            .expect("gam payload"),
        TxProofs::new_empty(),
    );
    prev = run_block(
        state,
        blocks,
        &prev,
        BlockComponents::new_txs_from_ol_transactions(vec![gam_tx]),
    );

    prev = run_block(state, blocks, &prev, BlockComponents::new_empty());

    let update_tx = build_snark_update(state, &inbox_msg);
    run_block(
        state,
        blocks,
        &prev,
        BlockComponents::new_txs_from_ol_transactions(vec![update_tx]),
    )
}

/// Builds a snark account update tx consuming the single inbox message from
/// `state`'s live snark account.
fn build_snark_update(
    state: &MemoryStateBaseLayer,
    inbox_msg: &MessageEntry,
) -> strata_ol_chain_types_new::OLTransaction {
    let snark_id = make_account_id(TEST_SNARK_ACCOUNT_ID);

    let mut inbox_tracker = InboxMmrTracker::new();
    let proof = inbox_tracker.add_message(inbox_msg);

    let (_, snark_state) = get_snark_state_expect(state, snark_id);
    SnarkUpdateBuilder::from_snark_state(snark_state.clone())
        .with_processed_msgs(vec![inbox_msg.clone()])
        .with_inbox_proofs(vec![proof])
        .with_transfer(make_account_id(TEST_RECIPIENT_ID), 1_000_000)
        .build(snark_id, make_state_root(2), vec![0u8; 32])
}
