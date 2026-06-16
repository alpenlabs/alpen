//! Epoch-building fixtures for checkpoint-sync consistency tests.
//!
//! [`build_epoch`] constructs one OL epoch (epoch 1, built on the genesis
//! epoch 0), runs it through the block-sync STF to capture reference values,
//! and derives the checkpoint payload a checkpoint-sync run would consume.
//! The resulting [`BuiltEpoch`] lets a test compare both reconstruction paths.

#![allow(unreachable_pub, reason = "test fixture module")]

use strata_acct_types::{BitcoinAmount, MessageEntry, RawMerkleProof};
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
use strata_ol_chain_types_new::{OLBlock, OLBlockHeader};
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
        make_deposit_manifest_for_account, make_genesis_state, make_state_root,
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
    /// Multiple OL txs, plus a terminal deposit manifest. Exercises per-update record
    /// reconstruction from `ol_logs` (N > 1) alongside deposit-manifest indexing.
    SnarkMultiUpdateAndDeposit,
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
}

impl BuiltEpoch {
    /// Returns the indexer writes captured by block-sync execution.
    pub fn block_sync_indexer_writes(&self) -> &IndexerWrites {
        &self.block_sync_indexer_writes
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
    let terminal_manifest = match shape {
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
            manifest
        }
        EpochShape::SnarkMultiUpdateAndDeposit => {
            let prev = run_snark_multi_update_blocks(&mut state, &mut blocks, genesis.header());
            let manifest = make_deposit_manifest_for_account(
                TERMINAL_L1_HEIGHT,
                0,
                snark_serial,
                SubjectId::from([42u8; 32]),
                BitcoinAmount::from_sat(150_000_000),
            );
            run_terminal(&mut state, &mut blocks, &prev, manifest.clone());
            manifest
        }
    };

    let terminal_block = blocks.last().expect("epoch has a terminal block").clone();
    let terminal_header = terminal_block.header().clone();

    // Run the epoch through the block-sync STF to capture reference values.
    let pre_epoch_layer = MemoryStateBaseLayer::new(pre_epoch_state.clone());
    let (block_sync_state, block_sync_state_root, block_sync_indexer_writes) =
        run_block_sync(&pre_epoch_layer, &blocks, genesis.header());

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

    let checkpoint_payload =
        assemble_checkpoint_payload(da_blob, ol_logs, &terminal_header, terminal_commitment);

    BuiltEpoch {
        epoch_commitment,
        prev_epoch_idx: 0,
        prev_summary,
        prev_terminal: genesis_commitment,
        pre_epoch_state,
        manifests_by_height: vec![(TERMINAL_L1_HEIGHT, terminal_manifest)],
        checkpoint_payload,
        block_sync_state_root,
        block_sync_summary,
        block_sync_indexer_writes,
    }
}

/// Assembles the checkpoint payload from the DA blob, OL logs, and terminal header.
fn assemble_checkpoint_payload(
    da_blob: Vec<u8>,
    ol_logs: Vec<CheckpointOLLog>,
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
        CheckpointSidecar::new(da_blob, ol_logs, complement).expect("build checkpoint sidecar");
    let tip = CheckpointTip::new(
        terminal_header.epoch(),
        TERMINAL_L1_HEIGHT,
        terminal_commitment,
    );
    CheckpointPayload::new(tip, sidecar, Vec::new()).expect("build checkpoint payload")
}

/// Runs the epoch's blocks through the block-sync STF, accumulating the write
/// batch and indexer writes across all blocks into a single pass.
///
/// Returns the post-epoch toplevel state, its state root, and the merged
/// indexer writes.
fn run_block_sync(
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
            BridgeParams::default(),
        )
        .expect("block-sync verify_block");
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

    (new_state.into_inner(), state_root, indexer_writes)
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

/// Runs the non-terminal blocks of a multi-update snark epoch: two GAMs
/// deliver two distinct inbox messages, then two snark updates each consume
/// one (in order). Returns the header of the last block.
fn run_snark_multi_update_blocks(
    state: &mut MemoryStateBaseLayer,
    blocks: &mut Vec<OLBlock>,
    genesis_header: &OLBlockHeader,
) -> OLBlockHeader {
    use strata_ol_chain_types_new::{OLTransaction, OLTransactionData, TxProofs};

    let snark_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let msg_a = snark_inbox_msg_with_data(b"multi-msg-a");
    let msg_b = snark_inbox_msg_with_data(b"multi-msg-b");

    // Two GAMs in successive blocks deliver two distinct inbox messages.
    let gam_a = OLTransaction::new(
        OLTransactionData::from_gam_bytes(snark_id, msg_a.payload().data().to_vec())
            .expect("gam payload"),
        TxProofs::new_empty(),
    );
    let gam_b = OLTransaction::new(
        OLTransactionData::from_gam_bytes(snark_id, msg_b.payload().data().to_vec())
            .expect("gam payload"),
        TxProofs::new_empty(),
    );
    let mut prev = run_block(
        state,
        blocks,
        genesis_header,
        BlockComponents::new_txs_from_ol_transactions(vec![gam_a]),
    );
    prev = run_block(
        state,
        blocks,
        &prev,
        BlockComponents::new_txs_from_ol_transactions(vec![gam_b]),
    );

    // Track the MMR across both adds, then read each leaf's *final* proof so
    // both are valid against the post-GAM-b MMR state the snark account sees.
    let mut tracker = InboxMmrTracker::new();
    tracker.add_message(&msg_a);
    tracker.add_message(&msg_b);
    let proof_a = tracker.proof_for(0);
    let proof_b = tracker.proof_for(1);

    // First update consumes msg_a; the snark account's seqno/next_inbox_idx
    // advance accordingly so the second update is built against the post-state.
    let update_a = build_snark_update_with(state, &msg_a, proof_a, make_state_root(2));
    prev = run_block(
        state,
        blocks,
        &prev,
        BlockComponents::new_txs_from_ol_transactions(vec![update_a]),
    );

    let update_b = build_snark_update_with(state, &msg_b, proof_b, make_state_root(3));
    run_block(
        state,
        blocks,
        &prev,
        BlockComponents::new_txs_from_ol_transactions(vec![update_b]),
    )
}

/// Helper to build a snark update tx with caller-supplied inbox proof and post-state root.
fn build_snark_update_with(
    state: &MemoryStateBaseLayer,
    inbox_msg: &MessageEntry,
    proof: RawMerkleProof,
    new_state_root: Buf32,
) -> strata_ol_chain_types_new::OLTransaction {
    let snark_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let (_, snark_state) = get_snark_state_expect(state, snark_id);
    SnarkUpdateBuilder::from_snark_state(snark_state.clone())
        .with_processed_msgs(vec![inbox_msg.clone()])
        .with_inbox_proofs(vec![proof])
        .with_transfer(make_account_id(TEST_RECIPIENT_ID), 1_000_000)
        .build(snark_id, new_state_root, vec![0u8; 32])
}
