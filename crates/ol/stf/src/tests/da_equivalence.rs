//! Tests that DA reconstruction produces the same state as direct block execution.

use std::collections::HashMap;

use strata_acct_types::{AccountId, BitcoinAmount};
use strata_checkpoint_types::EpochSummary;
use strata_identifiers::{Buf64, L1BlockCommitment, OLBlockCommitment};
use strata_ledger_types::{AccountTypeState, IStateAccessor, NewAccountData};
use strata_ol_chain_types_new::{OLBlock, OLBlockBody, OLBlockHeader, SignedOLBlockHeader};
use strata_ol_da::decode_ol_da_payload_bytes;
use strata_ol_state_support_types::{
    DaAccumulatingState, IndexerState, IndexerWrites, WriteTrackingState,
};
use strata_ol_state_types::OLSnarkAccountState;
use strata_predicate::PredicateKey;

use crate::{
    BlockInfo, EpochInitialContext, apply_da_epoch,
    assembly::{BlockComponents, CompletedBlock, execute_block_batch},
    test_utils::{
        build_chain_with_transactions, create_empty_account, create_test_genesis_state,
        get_test_recipient_account_id, get_test_snark_account_id, get_test_state_root,
        test_account_id,
    },
    verify_block,
};

/// Creates a genesis state with the same accounts that `build_chain_with_transactions` sets up.
fn create_state_matching_chain_setup() -> strata_ol_state_types::OLState {
    let mut state = create_test_genesis_state();

    // Mirror account creation from build_chain_with_transactions
    let snark_id = get_test_snark_account_id();
    let update_vk = PredicateKey::always_accept();
    let initial_state_root = get_test_state_root(1);
    let snark_state = OLSnarkAccountState::new_fresh(update_vk, initial_state_root);
    let balance = BitcoinAmount::from_sat(100_000_000);
    let new_acct_data = NewAccountData::new(balance, AccountTypeState::Snark(snark_state));
    state
        .create_new_account(snark_id, new_acct_data)
        .expect("create snark account");

    let gam_target = test_account_id(1);
    create_empty_account(&mut state, gam_target);
    let recipient_id = get_test_recipient_account_id();
    create_empty_account(&mut state, recipient_id);

    state
}

fn to_ol_block(header: &OLBlockHeader, body: &OLBlockBody) -> OLBlock {
    let signed = SignedOLBlockHeader::new(header.clone(), Buf64::zero());
    OLBlock::new(signed, body.clone())
}

/// Creates a state matching the chain setup and advances it past genesis execution.
fn create_post_genesis_state(genesis: &CompletedBlock) -> strata_ol_state_types::OLState {
    let mut state = create_state_matching_chain_setup();
    let genesis_ol = to_ol_block(genesis.header(), genesis.body());
    let tracking = WriteTrackingState::new_from_state(&state);
    let mut indexer = IndexerState::new(tracking);
    verify_block(&mut indexer, genesis_ol.header(), None, genesis_ol.body()).expect("genesis");
    let (tracking, _) = indexer.into_parts();
    let batch = tracking.into_batch();
    state.apply_write_batch(batch).expect("apply genesis");
    state
}

/// Executes a single epoch's blocks via `DaAccumulatingState`, extracts the DA blob,
/// then reconstructs state from that blob using the 3-step DA pipeline.
/// Asserts both paths produce the same state root.
#[test]
fn da_reconstruction_matches_block_execution() {
    let slots_per_epoch = 4;
    // genesis (terminal, epoch 0) + 4 blocks (epoch 1, terminal at slot 4) = 5 blocks
    let num_blocks = 5;

    let mut exec_state = create_test_genesis_state();
    let blocks = build_chain_with_transactions(&mut exec_state, num_blocks, slots_per_epoch);

    // Split: genesis is epoch 0, blocks[1..5] are epoch 1.
    let genesis = &blocks[0];
    let epoch_blocks: Vec<&CompletedBlock> = blocks[1..].iter().collect();
    let terminal = epoch_blocks.last().unwrap();

    assert!(genesis.header().is_terminal(), "genesis must be terminal");
    assert!(
        terminal.header().is_terminal(),
        "last block must be terminal"
    );

    // State root after full execution (reference).
    let exec_root = exec_state.compute_state_root().expect("exec state root");

    // -- Execute epoch 0 (genesis) on both DA-tracking and reconstruction states --
    let mut da_state = create_state_matching_chain_setup();
    let mut da_acc = DaAccumulatingState::new(da_state);
    let genesis_ol = to_ol_block(genesis.header(), genesis.body());
    verify_block(&mut da_acc, genesis_ol.header(), None, genesis_ol.body())
        .expect("genesis verify");

    let genesis_blob = da_acc
        .take_completed_epoch_da_blob()
        .expect("take genesis blob")
        .expect("genesis should produce a blob");
    da_state = da_acc.into_inner();

    // -- Execute epoch 1 blocks via DaAccumulatingState --
    let ol_blocks: Vec<OLBlock> = epoch_blocks
        .iter()
        .map(|b| to_ol_block(b.header(), b.body()))
        .collect();

    let mut da_acc = DaAccumulatingState::new(da_state);
    execute_block_batch(&mut da_acc, &ol_blocks, genesis.header()).expect("batch execution");

    let epoch1_blob = da_acc
        .take_completed_epoch_da_blob()
        .expect("take epoch1 blob")
        .expect("epoch 1 should produce a blob");
    da_state = da_acc.into_inner();

    // Sanity: DA-path execution matches direct execution.
    let da_exec_root = da_state.compute_state_root().expect("da exec state root");
    assert_eq!(
        exec_root, da_exec_root,
        "DA-tracked execution must match direct execution"
    );

    // -- Reconstruct entirely from DA blobs --
    // Pre-genesis account setup is not captured by DA blobs, so reconstruction
    // must start from the same initial state.
    let mut recon_state = create_state_matching_chain_setup();

    // Epoch 0 reconstruction
    let payload0 = decode_ol_da_payload_bytes(&genesis_blob).expect("decode genesis blob");
    let ep0_commitment = OLBlockCommitment::new(0, genesis.header().compute_blkid());
    let epctx0 = EpochInitialContext::new(0, ep0_commitment);
    let blkinfo0 = BlockInfo::new(genesis.header().timestamp(), 0, 0);
    let components0 = BlockComponents::from_block(&genesis_ol);
    apply_da_epoch(
        &epctx0,
        &mut recon_state,
        payload0,
        blkinfo0,
        components0.manifest_container(),
    )
    .expect("epoch 0 reconstruction");

    // Epoch 1 reconstruction
    let payload1 = decode_ol_da_payload_bytes(&epoch1_blob).expect("decode epoch1 blob");
    let terminal_slot = terminal.header().slot();
    let ep1_commitment = OLBlockCommitment::new(terminal_slot, terminal.header().compute_blkid());
    let epctx1 = EpochInitialContext::new(1, ep1_commitment);
    let blkinfo1 = BlockInfo::new(terminal.header().timestamp(), terminal_slot, 1);
    let terminal_ol = to_ol_block(terminal.header(), terminal.body());
    let components1 = BlockComponents::from_block(&terminal_ol);
    apply_da_epoch(
        &epctx1,
        &mut recon_state,
        payload1,
        blkinfo1,
        components1.manifest_container(),
    )
    .expect("epoch 1 reconstruction");

    let recon_root = recon_state.compute_state_root().expect("recon state root");
    assert_eq!(
        exec_root, recon_root,
        "DA reconstruction must match direct execution"
    );
}

/// Database-visible artifacts produced by epoch processing, comparable across paths.
struct EpochArtifacts {
    summary: EpochSummary,
    new_account_ids: Vec<AccountId>,
    snark_extra_data: HashMap<AccountId, Vec<Vec<u8>>>,
}

/// Extracts all snark extra_data per account from IndexerWrites.
fn extract_snark_extra_data(writes: &IndexerWrites) -> HashMap<AccountId, Vec<Vec<u8>>> {
    let mut map: HashMap<AccountId, Vec<Vec<u8>>> = HashMap::new();
    for update in writes.snark_state_updates() {
        if let Some(data) = update.extra_data() {
            map.entry(update.account_id())
                .or_default()
                .push(data.to_vec());
        }
    }
    map
}

/// Executes epoch 1 blocks one-by-one through `IndexerState<WriteTrackingState<_>>`,
/// collecting all DB-visible artifacts.
fn execute_epoch_via_blocks(
    genesis: &CompletedBlock,
    genesis_commitment: OLBlockCommitment,
    ol_blocks: &[OLBlock],
) -> EpochArtifacts {
    let mut state = create_post_genesis_state(genesis);
    let mut all_new_account_ids = Vec::new();
    let mut all_writes = IndexerWrites::new();
    let mut prev_header: Option<OLBlockHeader> = None;

    for (i, ol_block) in ol_blocks.iter().enumerate() {
        let parent = if i == 0 {
            Some(genesis.header())
        } else {
            prev_header.as_ref()
        };

        let tracking = WriteTrackingState::new_from_state(&state);
        let mut indexer = IndexerState::new(tracking);
        verify_block(&mut indexer, ol_block.header(), parent, ol_block.body())
            .expect("block verify");

        let (tracking, block_writes) = indexer.into_parts();
        let batch = tracking.into_batch();

        all_new_account_ids.extend(batch.ledger().new_accounts().to_vec());
        all_writes.extend(block_writes);

        prev_header = Some(ol_block.header().clone());
        state.apply_write_batch(batch).expect("apply batch");
    }

    let terminal = ol_blocks.last().expect("must have blocks");
    let terminal_slot = terminal.header().slot();
    let terminal_commitment =
        OLBlockCommitment::new(terminal_slot, terminal.header().compute_blkid());
    let state_root = state.compute_state_root().expect("state root");
    let l1 = L1BlockCommitment::new(state.last_l1_height(), *state.last_l1_blkid());
    let summary = EpochSummary::new(1, terminal_commitment, genesis_commitment, l1, state_root);

    all_new_account_ids.sort();
    EpochArtifacts {
        summary,
        new_account_ids: all_new_account_ids,
        snark_extra_data: extract_snark_extra_data(&all_writes),
    }
}

/// Gets the DA blob via `DaAccumulatingState`, then reconstructs epoch 1 state
/// through the 3-step DA pipeline wrapped in `IndexerState`, collecting all
/// DB-visible artifacts.
fn reconstruct_epoch_via_da(
    genesis: &CompletedBlock,
    genesis_commitment: OLBlockCommitment,
    ol_blocks: &[OLBlock],
) -> EpochArtifacts {
    // Execute through DaAccumulatingState to get the blob.
    let da_state = create_post_genesis_state(genesis);
    let mut da_acc = DaAccumulatingState::new(da_state);
    execute_block_batch(&mut da_acc, ol_blocks, genesis.header()).expect("batch");
    let blob = da_acc
        .take_completed_epoch_da_blob()
        .expect("take blob")
        .expect("blob");

    // Decode and extract new account IDs before the payload is consumed.
    let payload = decode_ol_da_payload_bytes(&blob).expect("decode blob");
    let mut new_account_ids: Vec<AccountId> = payload
        .state_diff
        .ledger
        .new_accounts
        .entries()
        .iter()
        .map(|e| e.account_id)
        .collect();

    // Reconstruct with IndexerState.
    let terminal = ol_blocks.last().expect("must have blocks");
    let terminal_slot = terminal.header().slot();
    let terminal_commitment =
        OLBlockCommitment::new(terminal_slot, terminal.header().compute_blkid());
    let epctx = EpochInitialContext::new(1, terminal_commitment);
    let blkinfo = BlockInfo::new(terminal.header().timestamp(), terminal_slot, 1);
    let components = BlockComponents::from_block(terminal);

    let recon_state = create_post_genesis_state(genesis);
    let mut indexer = IndexerState::new(recon_state);
    apply_da_epoch(
        &epctx,
        &mut indexer,
        payload,
        blkinfo,
        components.manifest_container(),
    )
    .expect("DA epoch reconstruction");

    let (state, writes) = indexer.into_parts();
    let state_root = state.compute_state_root().expect("state root");
    let l1 = L1BlockCommitment::new(state.last_l1_height(), *state.last_l1_blkid());
    let summary = EpochSummary::new(1, terminal_commitment, genesis_commitment, l1, state_root);

    new_account_ids.sort();
    EpochArtifacts {
        summary,
        new_account_ids,
        snark_extra_data: extract_snark_extra_data(&writes),
    }
}

/// Compares the database-visible artifacts produced by block execution vs DA
/// reconstruction for a single epoch.
#[test]
fn da_and_block_execution_produce_same_db_artifacts() {
    let slots_per_epoch = 4;
    let num_blocks = 5;

    let mut exec_state = create_test_genesis_state();
    let blocks = build_chain_with_transactions(&mut exec_state, num_blocks, slots_per_epoch);

    let genesis = &blocks[0];
    let epoch_blocks = &blocks[1..];

    assert!(genesis.header().is_terminal());
    assert!(epoch_blocks.last().unwrap().header().is_terminal());

    let ol_blocks: Vec<OLBlock> = epoch_blocks
        .iter()
        .map(|b| to_ol_block(b.header(), b.body()))
        .collect();

    let genesis_commitment = OLBlockCommitment::new(0, genesis.header().compute_blkid());

    let blk = execute_epoch_via_blocks(genesis, genesis_commitment, &ol_blocks);
    let da = reconstruct_epoch_via_da(genesis, genesis_commitment, &ol_blocks);

    // Sanity: artifacts are non-trivial (not comparing two empty sets).
    assert!(
        !blk.snark_extra_data.is_empty(),
        "block path should produce snark extra data"
    );

    assert_eq!(blk.summary, da.summary, "EpochSummary must match");
    assert_eq!(
        blk.new_account_ids, da.new_account_ids,
        "new account IDs must match"
    );
    assert_eq!(
        blk.snark_extra_data, da.snark_extra_data,
        "snark extra data per account must match"
    );
}
