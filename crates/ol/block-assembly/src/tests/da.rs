//! DA accumulation tests.
//!
//! Tests that verify DA is correctly accumulated during block assembly,
//! reset at epoch boundaries, and rolled back on failed transactions.

use std::sync::Arc;

use strata_identifiers::OLBlockCommitment;
use strata_ol_chain_types_new::{OLBlock, OLBlockHeader};
use strata_ol_state_support_types::{
    DaAccumulatingState, EpochDaAccumulator, MemoryStateBaseLayer,
};
use strata_ol_stf::execute_block_batch;

use crate::{
    context::BlockAssemblyAnchorContext,
    da_tracker::{AccumulatedDaData, rebuild_accumulated_da_upto},
    test_utils::{
        DEFAULT_ACCOUNT_BALANCE, MempoolSnarkTxBuilder, TestAccount, TestEnv,
        TestStorageFixtureBuilder, block_and_post_state_from_output, generate_message_entries,
        included_txids, test_account_id,
    },
};

/// Finalizes an accumulator against the given state and returns the encoded DA blob bytes.
fn finalize_da_to_bytes(accumulator: EpochDaAccumulator, state: MemoryStateBaseLayer) -> Vec<u8> {
    let mut da_state = DaAccumulatingState::new_with_accumulator(state, accumulator);
    da_state
        .take_completed_epoch_da_blob()
        .expect("finalize should succeed")
        .expect("should produce a blob")
}

/// Builds blocks from the env parent commitment up to (not including) `target_slot`, threading DA.
/// Returns `(final_commitment, accumulated_da, Vec<(block, post_state)>)`.
async fn build_blocks_with_da_and_artifacts(
    env: &mut TestEnv,
    target_slot: u64,
) -> (
    OLBlockCommitment,
    AccumulatedDaData,
    Vec<(OLBlock, MemoryStateBaseLayer)>,
) {
    let mut current_commitment = env.parent_commitment();
    let mut accumulated_da = AccumulatedDaData::new_empty();
    let mut artifacts = Vec::new();

    let start_slot = if current_commitment.is_null() {
        0
    } else {
        current_commitment.slot() + 1
    };

    for slot in start_slot..target_slot {
        let output = env
            .construct_empty_block_with_da(accumulated_da)
            .await
            .unwrap_or_else(|e| panic!("Block construction at slot {slot} failed: {e:?}"));
        let (block, post_state) = block_and_post_state_from_output(&output);
        let new_commitment = env.persist(&output).await;

        artifacts.push((block, post_state));
        accumulated_da = output.accumulated_da;
        current_commitment = new_commitment;
    }

    (current_commitment, accumulated_da, artifacts)
}

/// Core correctness: DA accumulated incrementally during block assembly must produce
/// the same encoded blob as DA rebuilt by replaying those same blocks.
#[tokio::test(flavor = "multi_thread")]
async fn test_da_incremental_matches_replay() {
    let env_builder = TestStorageFixtureBuilder::new()
        .with_parent_slot(0)
        .with_l1_manifest_height_range(1..=3);
    let (fixture, parent_commitment) = env_builder.build_fixture().await;
    let mut env = TestEnv::from_fixture(fixture, parent_commitment);

    // Build blocks 1..5, threading DA through each.
    let start_commitment = env.parent_commitment();
    let (_final_commitment, incremental_da, artifacts) =
        build_blocks_with_da_and_artifacts(&mut env, 5).await;

    // Get the post-state of the last block for finalization.
    let (_, last_post_state) = artifacts.last().unwrap();

    // Finalize incremental accumulator to bytes.
    let (incremental_acc, incremental_logs) = incremental_da.into_parts();
    let incremental_blob = finalize_da_to_bytes(incremental_acc, last_post_state.clone());

    // Replay: use DaAccumulatingState to re-execute all blocks.
    // Get parent state of first block (genesis post-state).
    let genesis_state = env
        .ctx()
        .fetch_state_for_tip(start_commitment)
        .await
        .unwrap()
        .unwrap();

    let blocks: Vec<&OLBlock> = artifacts.iter().map(|(block, _)| block).collect();
    let first_parent_header = artifacts[0].0.header();

    // Get the parent header (genesis header) from storage.
    let parent_blkid = *first_parent_header.parent_blkid();
    let parent_block = env
        .ctx()
        .fetch_ol_block(parent_blkid)
        .await
        .unwrap()
        .unwrap();
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
    let env_builder = TestStorageFixtureBuilder::new()
        .with_parent_slot(0)
        .with_l1_manifest_height_range(1..=3);
    let (fixture, parent_commitment) = env_builder.build_fixture().await;
    let mut env = TestEnv::from_fixture(fixture, parent_commitment);

    // Build blocks 1..10 (slots before the terminal block), threading DA.
    let (pre_terminal_commitment, epoch1_da, _epoch1_artifacts) =
        build_blocks_with_da_and_artifacts(&mut env, 10).await;

    // Epoch 1 DA accumulator should have slot changes from blocks 1-9.
    let (epoch1_acc, _) = epoch1_da.clone().into_parts();
    let epoch1_pre_terminal_state = env
        .ctx()
        .fetch_state_for_tip(pre_terminal_commitment)
        .await
        .unwrap()
        .unwrap();
    let epoch1_blob =
        finalize_da_to_bytes(epoch1_acc, Arc::unwrap_or_clone(epoch1_pre_terminal_state));

    // Build terminal block (slot 10) with epoch 1 DA.
    let terminal_output = env
        .construct_empty_block_with_da(epoch1_da)
        .await
        .expect("terminal block construction should succeed");

    // Store terminal block so we can build on it.
    env.persist(&terminal_output).await;

    // Build slot 11 (first block of epoch 2) with FRESH empty DA.
    let epoch2_output = env
        .construct_empty_block_with_da(AccumulatedDaData::new_empty())
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
    let source_account = test_account_id(3);
    let messages = generate_message_entries(2, source_account);

    let env_builder = TestStorageFixtureBuilder::new()
        .with_parent_slot(0)
        .with_l1_manifest_height_range(1..=3)
        .with_account(
            TestAccount::new(valid_account, DEFAULT_ACCOUNT_BALANCE).with_inbox(messages.clone()),
        )
        .with_account(TestAccount::new(invalid_account, DEFAULT_ACCOUNT_BALANCE));
    let (fixture, parent_commitment) = env_builder.build_fixture().await;
    let env = TestEnv::from_fixture(fixture, parent_commitment);

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

    let output_both = env
        .construct_block_with_da(
            vec![(valid_txid, valid_tx), (invalid_txid, invalid_tx)],
            AccumulatedDaData::new_empty(),
        )
        .await
        .expect("block construction should succeed");

    // Verify only valid tx was included.
    let included = included_txids(&output_both.template);
    assert_eq!(
        included,
        vec![valid_txid],
        "only valid tx should be included"
    );

    // Build a reference block with only the valid tx (same tx object via clone).
    let output_valid_only = env
        .construct_block(vec![(valid_txid, valid_tx_clone)])
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
    let env_builder = TestStorageFixtureBuilder::new()
        .with_parent_slot(0)
        .with_l1_manifest_height_range(1..=3);
    let (fixture, parent_commitment) = env_builder.build_fixture().await;
    let mut env = TestEnv::from_fixture(fixture, parent_commitment);

    // Build blocks 1..5, threading DA incrementally.
    let (final_commitment, incremental_da, artifacts) =
        build_blocks_with_da_and_artifacts(&mut env, 5).await;

    let (_, last_post_state) = artifacts.last().unwrap();

    // Finalize incremental accumulator.
    let (incremental_acc, incremental_logs) = incremental_da.into_parts();
    let incremental_blob = finalize_da_to_bytes(incremental_acc, last_post_state.clone());

    // Rebuild DA from scratch using the production code path.
    let epoch = artifacts[0].0.header().epoch();
    let rebuilt_da = rebuild_accumulated_da_upto(final_commitment, epoch, env.ctx())
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
