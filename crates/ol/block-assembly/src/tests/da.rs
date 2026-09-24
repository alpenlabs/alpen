//! DA accumulation tests.
//!
//! Tests that verify DA is correctly accumulated during block assembly,
//! reset at epoch boundaries, and rolled back on failed transactions.

use std::sync::Arc;

use strata_acct_types::{AccountId, AccountSerial, BitcoinAmount};
use strata_identifiers::{OLBlockCommitment, SubjectId};
use strata_ol_chain_types_v1::{OLBlockHeaderV1, OLBlockV1};
use strata_ol_params::OLRuntimeParams;
use strata_ol_state_support_types::{
    DaAccumulatingState, EpochDaAccumulator, MemoryStateBaseLayer,
};
use strata_ol_state_types::{
    IStateAccessor, IStateAccessorMut, NewAccountData, NewAccountTypeState,
};
use strata_ol_state_types_v1::OLStateV1;
use strata_ol_stf::{
    BlockInfo, EpochInfo, OLSpecId, apply_da_epoch, execute_block_batch_predrain, verify_block,
};
use strata_ol_stf_v1::test_utils::make_deposit_log_for_account;

use crate::context::BlockAssemblyAnchorContext;
use crate::resource_state::{
    AccumulatedDaData, EpochResourceState, rebuild_epoch_resource_state_upto,
};
use crate::test_utils::{
    DEFAULT_ACCOUNT_BALANCE, MempoolSnarkTxBuilder, TEST_SLOTS_PER_EPOCH, TestAccount, TestEnv,
    TestStorageFixtureBuilder, account_balance, block_and_post_state_from_output,
    create_test_genesis_state, generate_message_entries, included_txids, test_account_id,
};

/// Finalizes an accumulator against the given state and returns the encoded DA blob bytes.
fn finalize_da_to_bytes(
    accumulator: EpochDaAccumulator,
    state: MemoryStateBaseLayer<OLStateV1>,
) -> Vec<u8> {
    let mut da_state = DaAccumulatingState::new_with_accumulator(state, accumulator);
    da_state
        .take_completed_epoch_da_blob()
        .expect("finalize should succeed")
        .expect("should produce a blob")
}

/// Builds blocks from the env parent commitment up to (not including) `target_slot`, threading
/// resource state.
///
/// Returns `(final_commitment, resource_state, Vec<(block, post_state)>)`.
async fn build_blocks_with_resource_state_and_artifacts(
    env: &mut TestEnv,
    target_slot: u64,
) -> (
    OLBlockCommitment,
    EpochResourceState,
    Vec<(OLBlockV1, MemoryStateBaseLayer<OLStateV1>)>,
) {
    let mut current_commitment = env.parent_commitment();
    let mut resource_state = EpochResourceState::new_empty();
    let mut artifacts = Vec::new();

    let start_slot = if current_commitment.is_null() {
        0
    } else {
        current_commitment.slot() + 1
    };

    for slot in start_slot..target_slot {
        let output = env
            .construct_empty_block_with_resource_state(resource_state)
            .await
            .unwrap_or_else(|e| panic!("Block construction at slot {slot} failed: {e:?}"));
        let (block, post_state) = block_and_post_state_from_output(&output);
        let new_commitment = env.persist(&output).await;

        artifacts.push((block, post_state));
        resource_state = output.resource_state;
        current_commitment = new_commitment;
    }

    (current_commitment, resource_state, artifacts)
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
    let (_final_commitment, block_assembled_state, artifacts) =
        build_blocks_with_resource_state_and_artifacts(&mut env, 5).await;

    // Get the post-state of the last block for finalization.
    let (_, last_post_state) = artifacts.last().unwrap();

    // Finalize incremental accumulator to bytes.
    let (incremental_acc, incremental_logs) = block_assembled_state.da().clone().into_parts();
    let incremental_blob = finalize_da_to_bytes(incremental_acc, last_post_state.clone());

    // Replay: use DaAccumulatingState to re-execute all blocks.
    // Get parent state of first block (genesis post-state).
    let genesis_state = env
        .ctx()
        .fetch_state_for_tip(start_commitment)
        .await
        .unwrap()
        .unwrap();

    let blocks: Vec<&OLBlockV1> = artifacts.iter().map(|(block, _)| block).collect();
    let first_parent_header = artifacts[0].0.header();

    // Get the parent header (genesis header) from storage.
    let parent_blkid = *first_parent_header.parent_blkid();
    let parent_block = env
        .ctx()
        .fetch_ol_block(parent_blkid)
        .await
        .unwrap()
        .unwrap();
    let parent_header: &OLBlockHeaderV1 = parent_block.header();

    let owned_blocks: Vec<OLBlockV1> = blocks.into_iter().cloned().collect();
    let mut replay_da_state = DaAccumulatingState::new(Arc::unwrap_or_clone(genesis_state));
    let replay_logs = execute_block_batch_predrain(
        OLSpecId::V1,
        &mut replay_da_state,
        &owned_blocks,
        parent_header,
        &OLRuntimeParams::test_default(),
    )
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
    let (pre_terminal_commitment, epoch1_state, _epoch1_artifacts) =
        build_blocks_with_resource_state_and_artifacts(&mut env, 10).await;

    // Epoch 1 DA accumulator should have slot changes from blocks 1-9.
    let (epoch1_acc, _) = epoch1_state.da().clone().into_parts();
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
        .construct_empty_block_with_resource_state(epoch1_state)
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
    let (epoch2_acc, epoch2_logs) = epoch2_output.resource_state.da().clone().into_parts();
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
    let (acc_both, _) = output_both.resource_state.da().clone().into_parts();
    let blob_both = finalize_da_to_bytes(acc_both, output_both.post_state);

    let (acc_valid, _) = output_valid_only.resource_state.da().clone().into_parts();
    let blob_valid = finalize_da_to_bytes(acc_valid, output_valid_only.post_state);

    assert_eq!(
        blob_both, blob_valid,
        "DA with rolled-back failed tx must match DA with only valid tx"
    );
}

/// `rebuild_epoch_resource_state_upto` must produce the same DA as incremental accumulation.
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
    let (final_commitment, block_assembled_state, artifacts) =
        build_blocks_with_resource_state_and_artifacts(&mut env, 5).await;

    let (_, last_post_state) = artifacts.last().unwrap();

    // Finalize incremental accumulator.
    let (incremental_acc, incremental_logs) = block_assembled_state.da().clone().into_parts();
    let incremental_blob = finalize_da_to_bytes(incremental_acc, last_post_state.clone());

    // Rebuild DA from scratch using the production code path.
    let epoch = artifacts[0].0.header().epoch();
    let rebuilt_state = rebuild_epoch_resource_state_upto(
        final_commitment,
        epoch,
        OLRuntimeParams::test_default(),
        env.ctx(),
    )
    .await
    .expect("rebuild_epoch_resource_state_upto should succeed");

    let (rebuilt_acc, rebuilt_logs) = rebuilt_state.da().clone().into_parts();
    let rebuilt_blob = finalize_da_to_bytes(rebuilt_acc, last_post_state.clone());

    assert_eq!(
        incremental_blob, rebuilt_blob,
        "Rebuilt DA blob must match incrementally accumulated DA blob"
    );
    assert_eq!(
        incremental_logs, rebuilt_logs,
        "Rebuilt logs must match incrementally accumulated logs"
    );
    assert_eq!(
        block_assembled_state.manifest_count(),
        rebuilt_state.manifest_count(),
        "Rebuilt manifest count must match block-assembled manifest count"
    );
    assert!(
        block_assembled_state.manifest_count() > 0,
        "test fixture should exercise nonzero manifest-count rebuilding"
    );
}

/// Predicts the serial the fixture assigns to its first seeded account.
///
/// Manifest logs are fixed before the fixture seeds accounts, so a deposit log
/// needs its target serial in advance. The fixture creates accounts in order
/// on top of the test genesis state, so creating one account on a fresh
/// genesis state yields the same serial. Callers check the prediction with
/// [`TestStorageFixture::account_serial`](crate::test_utils::TestStorageFixture::account_serial).
fn predict_first_seeded_account_serial(account_id: AccountId) -> AccountSerial {
    create_test_genesis_state()
        .create_new_account(
            account_id,
            NewAccountData::new_empty(NewAccountTypeState::Empty),
        )
        .expect("probe account creation succeeds")
}

/// Blocks built by the sequencer pipeline must verify under the dispatched
/// STF, and the sequencer's incrementally accumulated epoch DA must match a
/// rebuild and reproduce the terminal state root on replay.
///
/// The epoch carries a snark account update that needs inbox proofs and a
/// deposit manifest whose effect only lands at the terminal drain.
#[tokio::test(flavor = "multi_thread")]
async fn test_sequencer_epoch_verifies_and_replays_from_da() {
    let snark_account = test_account_id(1);
    let messages = generate_message_entries(2, test_account_id(3));
    let deposit_amount = BitcoinAmount::try_from(150_000_000)
        .expect("amount must not exceed the Bitcoin money supply");
    let deposit_serial = predict_first_seeded_account_serial(snark_account);
    let deposit =
        make_deposit_log_for_account(deposit_serial, SubjectId::from([9u8; 32]), deposit_amount);
    let (fixture, genesis_commitment) = TestStorageFixtureBuilder::new()
        .with_parent_slot(0)
        .with_l1_manifest_height_range(1..=3)
        .with_l1_manifest_logs(3, [deposit])
        .with_account(
            TestAccount::new(snark_account, DEFAULT_ACCOUNT_BALANCE).with_inbox(messages.clone()),
        )
        .build_fixture()
        .await;
    assert_eq!(
        fixture.account_serial(snark_account),
        deposit_serial,
        "the deposit must target the seeded snark account"
    );
    let mut env = TestEnv::from_fixture(fixture, genesis_commitment);
    let runtime_params = OLRuntimeParams::test_default();

    // Build the epoch: a snark update first, then empty blocks until the
    // sealing policy ends the epoch.
    let update_tx = MempoolSnarkTxBuilder::new(snark_account)
        .with_seq_no(0)
        .with_processed_messages(messages)
        .build();
    let update_txid = update_tx.compute_txid();
    let mut output = env
        .construct_block_with_resource_state(
            [(update_txid, update_tx)],
            EpochResourceState::new_empty(),
        )
        .await
        .expect("snark update block constructs");
    assert_eq!(included_txids(&output.template), vec![update_txid]);

    let mut blocks = Vec::new();
    loop {
        let (block, _) = block_and_post_state_from_output(&output);
        let is_terminal = block.header().is_terminal();
        blocks.push(block);
        env.persist(&output).await;
        if is_terminal {
            break;
        }
        assert!(
            (blocks.len() as u64) < TEST_SLOTS_PER_EPOCH,
            "the sealing policy must end the epoch within {TEST_SLOTS_PER_EPOCH} blocks"
        );
        output = env
            .construct_empty_block_with_resource_state(output.resource_state)
            .await
            .expect("empty block constructs");
    }
    let terminal_commitment = env.parent_commitment();
    let terminal_header = blocks.last().expect("epoch has blocks").header().clone();
    let epoch_resource_state = output.resource_state;
    assert!(
        blocks
            .iter()
            .any(|block| block.body().manifests().is_some()),
        "the epoch must carry the deposit manifest"
    );

    let genesis_state = env
        .ctx()
        .fetch_state_for_tip(genesis_commitment)
        .await
        .expect("genesis state lookup")
        .expect("genesis state exists");
    let genesis_header = env
        .ctx()
        .fetch_ol_header(*genesis_commitment.blkid())
        .await
        .expect("genesis header lookup")
        .expect("genesis header exists");

    // Verification reproduces every header, including the logs commitment.
    let mut verified_state = Arc::unwrap_or_clone(genesis_state.clone());
    let mut parent = genesis_header.clone();
    let mut verified_logs = Vec::new();
    for block in &blocks {
        let logs = verify_block(
            OLSpecId::V1,
            &mut verified_state,
            block.header(),
            Some(&parent),
            block.body(),
            &runtime_params,
        )
        .expect("sequencer block verifies");
        verified_logs.extend(logs);
        parent = block.header().clone();
    }
    let expected_balance_sats =
        account_balance(genesis_state.as_ref(), snark_account).to_sat() + deposit_amount.to_sat();
    assert_eq!(
        account_balance(&verified_state, snark_account).to_sat(),
        expected_balance_sats,
        "the terminal drain must credit exactly the deposit"
    );

    // The sequencer accumulates DA before the drain. Finalize both
    // accumulators against the pre-drain epoch state, since new-account
    // entries read their contents from the supplied state.
    let mut predrain_state = Arc::unwrap_or_clone(genesis_state.clone());
    execute_block_batch_predrain(
        OLSpecId::V1,
        &mut predrain_state,
        &blocks,
        &genesis_header,
        &runtime_params,
    )
    .expect("pre-drain replay succeeds");

    let (incremental_acc, incremental_logs) = epoch_resource_state.da().clone().into_parts();
    let incremental_blob = finalize_da_to_bytes(incremental_acc, predrain_state.clone());
    let rebuilt_state = rebuild_epoch_resource_state_upto(
        terminal_commitment,
        terminal_header.epoch(),
        runtime_params,
        env.ctx(),
    )
    .await
    .expect("epoch resource state rebuilds");
    let (rebuilt_acc, rebuilt_logs) = rebuilt_state.da().clone().into_parts();
    let rebuilt_blob = finalize_da_to_bytes(rebuilt_acc, predrain_state);

    assert_eq!(incremental_blob, rebuilt_blob);
    assert_eq!(incremental_logs, rebuilt_logs);
    // The drain emits no OL logs, so the pre-drain logs are the block logs.
    assert_eq!(incremental_logs, verified_logs);

    // Replaying the DA and the epoch's manifests reproduces the terminal root.
    let manifests: Vec<_> = blocks
        .iter()
        .filter_map(|block| block.body().manifests())
        .flat_map(|container| container.manifests().iter().cloned())
        .collect();
    let epoch_info = EpochInfo::new(
        BlockInfo::from_header(&terminal_header),
        genesis_header.compute_block_commitment(),
    );
    let mut replayed_state = Arc::unwrap_or_clone(genesis_state);
    apply_da_epoch(
        OLSpecId::V1,
        &mut replayed_state,
        &epoch_info,
        &incremental_blob,
        &manifests,
        &runtime_params,
    )
    .expect("sequencer epoch DA replays");
    assert_eq!(
        replayed_state.compute_state_root().expect("state root"),
        *terminal_header.state_root()
    );
}
