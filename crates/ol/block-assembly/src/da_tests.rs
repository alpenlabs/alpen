//! DA accumulation tests.
//!
//! Tests that verify DA is correctly accumulated during block assembly,
//! reset at epoch boundaries, and rolled back on failed transactions.

use std::sync::Arc;

use strata_identifiers::{Buf64, OLBlockCommitment};
use strata_ol_chain_types_new::{OLBlock, OLBlockHeader, SignedOLBlockHeader};
use strata_ol_state_support_types::{DaAccumulatingState, EpochDaAccumulator};
use strata_ol_state_types::OLState;
use strata_ol_stf::execute_block_batch;
use strata_storage::NodeStorage;

use crate::{
    AccumulatorProofGenerator, EpochSealingPolicy,
    block_assembly::{calculate_block_slot_and_epoch, construct_block},
    context::BlockAssemblyAnchorContext,
    da_tracker::{AccumulatedDaData, rebuild_accumulated_da_upto},
    test_utils::{
        DEFAULT_ACCOUNT_BALANCE, MempoolSnarkTxBuilder, StorageInboxMmr, TestEnvBuilder,
        create_test_block_assembly_context, generate_message_entries,
        insert_inbox_messages_into_storage_state, test_account_id,
    },
    types::BlockGenerationConfig,
};

/// Finalizes an accumulator against the given state and returns the encoded DA blob bytes.
fn finalize_da_to_bytes(accumulator: EpochDaAccumulator, state: OLState) -> Vec<u8> {
    let mut da_state = DaAccumulatingState::new_with_accumulator(state, accumulator);
    da_state
        .take_completed_epoch_da_blob()
        .expect("finalize should succeed")
        .expect("should produce a blob")
}

/// Builds blocks from `start_commitment` up to (not including) `target_slot`, threading DA.
/// Returns `(final_commitment, accumulated_da, Vec<(block, post_state)>)`.
async fn build_blocks_with_da_and_artifacts<C, E>(
    start_commitment: OLBlockCommitment,
    target_slot: u64,
    ctx: &C,
    storage: &NodeStorage,
    epoch_sealing_policy: &E,
) -> (
    OLBlockCommitment,
    AccumulatedDaData,
    Vec<(OLBlock, OLState)>,
)
where
    C: BlockAssemblyAnchorContext<State = OLState> + AccumulatorProofGenerator,
    E: EpochSealingPolicy,
{
    let mut current_commitment = start_commitment;
    let mut accumulated_da = AccumulatedDaData::new_empty();
    let mut artifacts = Vec::new();

    let start_slot = if current_commitment.is_null() {
        0
    } else {
        start_commitment.slot() + 1
    };

    for slot in start_slot..target_slot {
        let config = BlockGenerationConfig::new(current_commitment);

        let parent_state = ctx
            .fetch_state_for_tip(config.parent_block_commitment())
            .await
            .unwrap_or_else(|e| panic!("Failed to fetch parent state at slot {slot}: {e:?}"))
            .unwrap_or_else(|| panic!("Missing parent state at slot {slot}"));

        let (block_slot, block_epoch) = calculate_block_slot_and_epoch(
            &config.parent_block_commitment(),
            parent_state.as_ref(),
        );

        let output = construct_block(
            ctx,
            epoch_sealing_policy,
            &config,
            parent_state,
            block_slot,
            block_epoch,
            vec![],
            accumulated_da,
        )
        .await
        .unwrap_or_else(|e| panic!("Block construction at slot {slot} failed: {e:?}"));

        let header = output.template.header();
        let new_commitment = OLBlockCommitment::new(header.slot(), header.compute_blkid());

        let signed_header = SignedOLBlockHeader::new(header.clone(), Buf64::zero());
        let block = OLBlock::new(signed_header, output.template.body().clone());

        storage
            .ol_block()
            .put_block_data_async(block.clone())
            .await
            .unwrap_or_else(|e| panic!("Failed to store block at slot {slot}: {e:?}"));

        storage
            .ol_state()
            .put_toplevel_ol_state_async(new_commitment, output.post_state.clone())
            .await
            .unwrap_or_else(|e| panic!("Failed to store state at slot {slot}: {e:?}"));

        artifacts.push((block, output.post_state));
        accumulated_da = output.accumulated_da;
        current_commitment = new_commitment;
    }

    (current_commitment, accumulated_da, artifacts)
}

/// Collects stored blocks for an epoch by walking forward from start_slot to end_slot (exclusive).
fn collect_blocks_from_artifacts(artifacts: &[(OLBlock, OLState)]) -> Vec<&OLBlock> {
    artifacts.iter().map(|(block, _)| block).collect()
}

/// Core correctness: DA accumulated incrementally during block assembly must produce
/// the same encoded blob as DA rebuilt by replaying those same blocks.
#[tokio::test(flavor = "multi_thread")]
async fn test_da_incremental_matches_replay() {
    let env = TestEnvBuilder::new()
        .with_parent_slot(0)
        .with_asm_manifests(&[1, 2, 3])
        .build()
        .await;

    let (ctx, _mempool) = create_test_block_assembly_context(env.storage.clone());

    // Build blocks 1..5, threading DA through each.
    let (_final_commitment, incremental_da, artifacts) = build_blocks_with_da_and_artifacts(
        env.parent_commitment,
        5,
        &ctx,
        env.storage.as_ref(),
        &env.epoch_sealing_policy,
    )
    .await;

    // Get the post-state of the last block for finalization.
    let (_, last_post_state) = artifacts.last().unwrap();

    // Finalize incremental accumulator to bytes.
    let (incremental_acc, incremental_logs) = incremental_da.into_parts();
    let incremental_blob = finalize_da_to_bytes(incremental_acc, last_post_state.clone());

    // Replay: use DaAccumulatingState to re-execute all blocks.
    // Get parent state of first block (genesis post-state).
    let genesis_state = ctx
        .fetch_state_for_tip(env.parent_commitment)
        .await
        .unwrap()
        .unwrap();

    let blocks: Vec<&OLBlock> = collect_blocks_from_artifacts(&artifacts);
    let first_parent_header = artifacts[0].0.header();

    // Get the parent header (genesis header) from storage.
    let parent_blkid = *first_parent_header.parent_blkid();
    let parent_block = ctx.fetch_ol_block(parent_blkid).await.unwrap().unwrap();
    let parent_header: &OLBlockHeader = parent_block.header();

    let owned_blocks: Vec<OLBlock> = blocks.into_iter().cloned().collect();
    let mut replay_da_state = DaAccumulatingState::new(Arc::unwrap_or_clone(genesis_state));
    let replay_logs = execute_block_batch(&mut replay_da_state, &owned_blocks, parent_header)
        .expect("replay should succeed");

    let (replay_acc, replay_inner) = replay_da_state.into_parts();
    let replay_blob = finalize_da_to_bytes(replay_acc, replay_inner);

    // The encoded DA blobs must be byte-identical.
    assert_eq!(
        incremental_blob, replay_blob,
        "Incremental DA blob must match replayed DA blob"
    );

    // Logs must also match.
    assert_eq!(
        incremental_logs, replay_logs,
        "Incremental logs must match replayed logs"
    );
}

/// DA must reset at epoch boundaries. Building blocks in epoch 2 with a fresh
/// accumulator should produce different DA than continuing with epoch 1's
/// accumulated data, proving that epoch DA is scoped correctly.
#[tokio::test(flavor = "multi_thread")]
async fn test_da_resets_at_epoch_boundary() {
    let env = TestEnvBuilder::new()
        .with_parent_slot(0)
        .with_asm_manifests(&[1, 2, 3])
        .build()
        .await;

    let (ctx, _mempool) = create_test_block_assembly_context(env.storage.clone());

    // Build blocks 1..10 (slots before the terminal block), threading DA.
    let (pre_terminal_commitment, epoch1_da, _epoch1_artifacts) =
        build_blocks_with_da_and_artifacts(
            env.parent_commitment,
            10,
            &ctx,
            env.storage.as_ref(),
            &env.epoch_sealing_policy,
        )
        .await;

    // Epoch 1 DA accumulator should have slot changes from blocks 1-9.
    let (epoch1_acc, _) = epoch1_da.clone().into_parts();
    let epoch1_pre_terminal_state = ctx
        .fetch_state_for_tip(pre_terminal_commitment)
        .await
        .unwrap()
        .unwrap();
    let epoch1_blob = finalize_da_to_bytes(epoch1_acc, (*epoch1_pre_terminal_state).clone());

    // Build terminal block (slot 10) with epoch 1 DA.
    let config = BlockGenerationConfig::new(pre_terminal_commitment);
    let (block_slot, block_epoch) = calculate_block_slot_and_epoch(
        &pre_terminal_commitment,
        epoch1_pre_terminal_state.as_ref(),
    );
    let terminal_output = construct_block(
        &ctx,
        &env.epoch_sealing_policy,
        &config,
        epoch1_pre_terminal_state,
        block_slot,
        block_epoch,
        vec![],
        epoch1_da,
    )
    .await
    .expect("terminal block construction should succeed");

    // Store terminal block so we can build on it.
    let terminal_header = terminal_output.template.header();
    let terminal_commitment =
        OLBlockCommitment::new(terminal_header.slot(), terminal_header.compute_blkid());
    let signed = SignedOLBlockHeader::new(terminal_header.clone(), Buf64::zero());
    let terminal_block = OLBlock::new(signed, terminal_output.template.body().clone());
    env.storage
        .ol_block()
        .put_block_data_async(terminal_block)
        .await
        .unwrap();
    env.storage
        .ol_state()
        .put_toplevel_ol_state_async(terminal_commitment, terminal_output.post_state.clone())
        .await
        .unwrap();

    // Build slot 11 (first block of epoch 2) with FRESH empty DA.
    let config_11 = BlockGenerationConfig::new(terminal_commitment);
    let parent_state_11 = ctx
        .fetch_state_for_tip(terminal_commitment)
        .await
        .unwrap()
        .unwrap();
    let (slot_11, epoch_11) =
        calculate_block_slot_and_epoch(&terminal_commitment, parent_state_11.as_ref());
    let epoch2_output = construct_block(
        &ctx,
        &env.epoch_sealing_policy,
        &config_11,
        parent_state_11,
        slot_11,
        epoch_11,
        vec![],
        AccumulatedDaData::new_empty(),
    )
    .await
    .expect("epoch 2 block construction should succeed");

    // Epoch 2 DA should only contain slot 11's changes, not epoch 1's.
    let (epoch2_acc, epoch2_logs) = epoch2_output.accumulated_da.into_parts();
    let epoch2_blob = finalize_da_to_bytes(epoch2_acc, epoch2_output.post_state);

    // The two blobs must differ: epoch 1 accumulated 9 slot changes, epoch 2 has 1.
    assert_ne!(
        epoch1_blob, epoch2_blob,
        "Epoch 2 DA should differ from epoch 1 DA (different slot ranges)"
    );

    // Epoch 2 logs should be empty (no txs, no manifests in non-terminal block).
    assert!(
        epoch2_logs.is_empty(),
        "First block of new epoch with no txs should have no logs"
    );
}

/// Failed transactions must not pollute the DA accumulator. Only successful
/// transaction mutations should appear in the final DA blob.
#[tokio::test(flavor = "multi_thread")]
async fn test_da_rollback_on_failed_tx() {
    let valid_account = test_account_id(1);
    let invalid_account = test_account_id(2);

    let env = TestEnvBuilder::new()
        .with_parent_slot(0)
        .with_asm_manifests(&[1, 2, 3])
        .with_account(valid_account, DEFAULT_ACCOUNT_BALANCE)
        .with_account(invalid_account, DEFAULT_ACCOUNT_BALANCE)
        .build()
        .await;

    // Setup inbox messages for valid account only.
    let source_account = test_account_id(3);
    let messages = generate_message_entries(2, source_account);
    let mut inbox_mmr = StorageInboxMmr::new(&env.storage, valid_account);
    inbox_mmr.add_messages(messages.clone());

    insert_inbox_messages_into_storage_state(
        env.storage.as_ref(),
        env.parent_commitment,
        valid_account,
        &messages,
    )
    .await;

    let (ctx, _mempool) = create_test_block_assembly_context(env.storage.clone());

    // Build the valid tx once and clone for reuse.
    let valid_tx = MempoolSnarkTxBuilder::new(valid_account)
        .with_seq_no(0)
        .with_processed_messages(messages)
        .build();
    let valid_txid = valid_tx.compute_txid();
    let valid_tx_clone = valid_tx.clone();

    // Invalid tx: wrong seq_no (expects 0 but we use 99).
    let invalid_tx = MempoolSnarkTxBuilder::new(invalid_account)
        .with_seq_no(99)
        .build();
    let invalid_txid = invalid_tx.compute_txid();

    let parent_state = ctx
        .fetch_state_for_tip(env.parent_commitment)
        .await
        .unwrap()
        .unwrap();

    let config = BlockGenerationConfig::new(env.parent_commitment);
    let (block_slot, block_epoch) =
        calculate_block_slot_and_epoch(&env.parent_commitment, parent_state.as_ref());

    // Build block with both valid + invalid txs.
    let output_both = construct_block(
        &ctx,
        &env.epoch_sealing_policy,
        &config,
        parent_state.clone(),
        block_slot,
        block_epoch,
        vec![(valid_txid, valid_tx), (invalid_txid, invalid_tx)],
        AccumulatedDaData::new_empty(),
    )
    .await
    .expect("block construction should succeed");

    // Verify only valid tx was included.
    let txs = output_both
        .template
        .body()
        .tx_segment()
        .expect("should have txs")
        .txs();
    assert_eq!(txs.len(), 1, "only valid tx should be included");
    assert_eq!(
        txs[0].target(),
        Some(valid_account),
        "included tx should target valid account"
    );

    // Build a reference block with only the valid tx (same tx object via clone).
    let output_valid_only = construct_block(
        &ctx,
        &env.epoch_sealing_policy,
        &config,
        parent_state,
        block_slot,
        block_epoch,
        vec![(valid_txid, valid_tx_clone)],
        AccumulatedDaData::new_empty(),
    )
    .await
    .expect("valid-only block construction should succeed");

    // Finalize both DA accumulators and compare.
    let (acc_both, _) = output_both.accumulated_da.into_parts();
    let blob_both = finalize_da_to_bytes(acc_both, output_both.post_state);

    let (acc_valid, _) = output_valid_only.accumulated_da.into_parts();
    let blob_valid = finalize_da_to_bytes(acc_valid, output_valid_only.post_state);

    assert_eq!(
        blob_both, blob_valid,
        "DA with rolled-back failed tx must match DA with only valid tx"
    );
}

/// `rebuild_accumulated_da_upto` must produce the same DA as incremental accumulation.
///
/// This exercises the `collect_epoch_blocks_until` -> `execute_block_batch` path,
/// which previously had a bug where the first epoch block's header was passed as the
/// parent header instead of the actual epoch boundary block's header.
#[tokio::test(flavor = "multi_thread")]
async fn test_rebuild_da_matches_incremental() {
    let env = TestEnvBuilder::new()
        .with_parent_slot(0)
        .with_asm_manifests(&[1, 2, 3])
        .build()
        .await;

    let (ctx, _mempool) = create_test_block_assembly_context(env.storage.clone());

    // Build blocks 1..5, threading DA incrementally.
    let (final_commitment, incremental_da, artifacts) = build_blocks_with_da_and_artifacts(
        env.parent_commitment,
        5,
        &ctx,
        env.storage.as_ref(),
        &env.epoch_sealing_policy,
    )
    .await;

    let (_, last_post_state) = artifacts.last().unwrap();

    // Finalize incremental accumulator.
    let (incremental_acc, incremental_logs) = incremental_da.into_parts();
    let incremental_blob = finalize_da_to_bytes(incremental_acc, last_post_state.clone());

    // Rebuild DA from scratch using the production code path.
    let epoch = artifacts[0].0.header().epoch();
    let rebuilt_da = rebuild_accumulated_da_upto(final_commitment, epoch, &ctx)
        .await
        .expect("rebuild_accumulated_da_upto should succeed");

    let (rebuilt_acc, rebuilt_logs) = rebuilt_da.into_parts();
    let rebuilt_blob = finalize_da_to_bytes(rebuilt_acc, last_post_state.clone());

    assert_eq!(
        incremental_blob, rebuilt_blob,
        "Rebuilt DA blob must match incrementally accumulated DA blob"
    );
    assert_eq!(
        incremental_logs, rebuilt_logs,
        "Rebuilt logs must match incrementally accumulated logs"
    );
}
