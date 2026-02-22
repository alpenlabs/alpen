//! Tests for ledger references (referencing ASM manifests)

use strata_acct_types::{AccountId, AcctError, BitcoinAmount, tree_hash::TreeHash};
use strata_asm_common::AsmManifest;
use strata_identifiers::{Buf32, WtxidsRoot};
use strata_ledger_types::{IAccountState, IStateAccessor};
use strata_ol_chain_types_new::{SnarkAccountUpdateTxPayload, TransactionPayload};
use strata_ol_state_types::OLState;
use strata_snark_acct_types::{
    AccumulatorClaim, LedgerRefProofs, LedgerRefs, MmrEntryProof, OutputTransfer, ProofState,
    SnarkAccountUpdate, SnarkAccountUpdateContainer, UpdateAccumulatorProofs, UpdateOperationData,
    UpdateOutputs,
};

use crate::{
    assembly::BlockComponents,
    context::BlockInfo,
    errors::ExecError,
    test_utils::{
        ManifestMmrTracker, create_empty_account, create_test_genesis_state,
        execute_block_with_outputs, execute_tx_in_block, get_test_recipient_account_id,
        get_test_snark_account_id, get_test_state_root, setup_genesis_with_snark_account,
        test_l1_block_id,
    },
};

#[test]
fn test_snark_update_with_valid_ledger_reference() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let recipient_id = get_test_recipient_account_id();

    // Setup: genesis with snark account
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

    // Create recipient account
    create_empty_account(&mut state, recipient_id);

    // Create parallel MMR tracker for manifests
    let mut manifest_tracker = ManifestMmrTracker::new();

    // Step 1: Execute a block with an ASM manifest to populate the state MMR
    let manifest1 = AsmManifest::new(
        1,
        test_l1_block_id(1),
        WtxidsRoot::from(Buf32::from([1u8; 32])),
        vec![], // No logs for simplicity
    );

    // Get the manifest hash before execution
    let manifest1_hash = <AsmManifest as TreeHash>::tree_hash_root(&manifest1);

    // Execute block with manifest
    let block1_info = BlockInfo::new(1001000, 1, 0); // slot 1, epoch 0
    let block1_components = BlockComponents::new_manifests(vec![manifest1.clone()]);
    let block1_output = execute_block_with_outputs(
        &mut state,
        &block1_info,
        Some(genesis_block.header()),
        block1_components,
    )
    .expect("Block 1 should execute");

    // Track the manifest in parallel MMR after execution (matching what state did)
    let (manifest1_index, manifest1_proof) = manifest_tracker.add_manifest(&manifest1);

    // Verify the manifest was added to state MMR
    assert_eq!(
        state.asm_manifests_mmr().num_entries(),
        manifest_tracker.num_entries(),
        "State MMR should match tracker MMR"
    );
    assert_eq!(manifest1_index, 0, "First manifest should be at index 0");

    // Step 2: Create a snark update that references the manifest
    // AccumulatorClaim.idx is L1 block height; offset = genesis_height(0) + 1 = 1
    let manifest1_height = manifest1_index + 1;
    let ledger_refs = LedgerRefs::new(vec![AccumulatorClaim::new(
        manifest1_height,
        manifest1_hash.into_inner(),
    )]);

    // Create update with ledger reference and a transfer
    let transfer = OutputTransfer::new(recipient_id, BitcoinAmount::from_sat(10_000_000));
    let update_outputs = UpdateOutputs::new(vec![transfer], vec![]);

    let seq_no = 0u64;
    let new_proof_state = ProofState::new(get_test_state_root(2), 0);
    let operation_data = UpdateOperationData::new(
        seq_no,
        new_proof_state,
        vec![],
        ledger_refs,
        update_outputs,
        vec![],
    );

    let base_update = SnarkAccountUpdate::new(operation_data, vec![0u8; 32]);

    // Include the valid proof for the ledger reference
    let accumulator_proofs =
        UpdateAccumulatorProofs::new(vec![], LedgerRefProofs::new(vec![manifest1_proof]));

    let update_container = SnarkAccountUpdateContainer::new(base_update, accumulator_proofs);
    let tx = TransactionPayload::SnarkAccountUpdate(SnarkAccountUpdateTxPayload::new(
        snark_id,
        update_container,
    ));

    // Step 3: Execute the update
    let (slot, epoch) = (2, 1); // Increment epoch because we processed manifests in last block
    let result = execute_tx_in_block(
        &mut state,
        block1_output.completed_block().header(),
        tx,
        slot,
        epoch,
    );

    assert!(
        result.is_ok(),
        "Update with valid ledger reference should succeed: {:?}",
        result.err()
    );

    // Verify the transfer was applied
    let snark_account = state.get_account_state(snark_id).unwrap().unwrap();
    assert_eq!(
        snark_account.balance(),
        BitcoinAmount::from_sat(90_000_000),
        "Sender balance should be reduced"
    );

    let recipient = state.get_account_state(recipient_id).unwrap().unwrap();
    assert_eq!(
        recipient.balance(),
        BitcoinAmount::from_sat(10_000_000),
        "Recipient should receive transfer"
    );
}

#[test]
fn test_snark_update_with_invalid_ledger_reference() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();

    // Setup: genesis with snark account
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

    // Create parallel MMR tracker
    let mut manifest_tracker = ManifestMmrTracker::new();

    // Step 1: Execute a block with an ASM manifest
    let manifest1 = AsmManifest::new(
        1,
        test_l1_block_id(1),
        WtxidsRoot::from(Buf32::from([1u8; 32])),
        vec![],
    );

    // Get the manifest hash before execution
    let manifest1_hash = <AsmManifest as TreeHash>::tree_hash_root(&manifest1);

    // Execute block with manifest
    let block1_info = BlockInfo::new(1001000, 1, 0); // slot 1, epoch 0
    let block1_components = BlockComponents::new_manifests(vec![manifest1.clone()]);
    let block1_output = execute_block_with_outputs(
        &mut state,
        &block1_info,
        Some(genesis_block.header()),
        block1_components,
    )
    .expect("Block 1 should execute");

    // Track the manifest in parallel MMR after execution (matching what state did)
    let (manifest1_index, _valid_proof) = manifest_tracker.add_manifest(&manifest1);

    // Step 2: Create a snark update with INVALID ledger reference proof
    // AccumulatorClaim.idx is L1 block height; offset = genesis_height(0) + 1 = 1
    let manifest1_height = manifest1_index + 1;
    let ledger_refs = LedgerRefs::new(vec![AccumulatorClaim::new(
        manifest1_height,
        manifest1_hash.into_inner(),
    )]);

    // Create an invalid proof with wrong cohashes
    let invalid_proof = MmrEntryProof::new(
        manifest1_hash.into_inner(),
        strata_acct_types::MerkleProof::from_cohashes(
            vec![[0xff; 32]], // Invalid cohash
            manifest1_index,  // proof uses raw MMR index
        ),
    );

    // Create update with ledger reference
    let update_outputs = UpdateOutputs::new_empty();

    let seq_no = 0u64;
    let new_proof_state = ProofState::new(get_test_state_root(2), 0);
    let operation_data = UpdateOperationData::new(
        seq_no,
        new_proof_state,
        vec![],
        ledger_refs,
        update_outputs,
        vec![],
    );

    let base_update = SnarkAccountUpdate::new(operation_data, vec![0u8; 32]);

    // Include the INVALID proof
    let accumulator_proofs =
        UpdateAccumulatorProofs::new(vec![], LedgerRefProofs::new(vec![invalid_proof]));

    let update_container = SnarkAccountUpdateContainer::new(base_update, accumulator_proofs);
    let tx = TransactionPayload::SnarkAccountUpdate(SnarkAccountUpdateTxPayload::new(
        snark_id,
        update_container,
    ));

    // Step 3: Execute and expect failure
    let (slot, epoch) = (2, 1); // Increment epoch because we processed manifests in the last block
    let result = execute_tx_in_block(
        &mut state,
        block1_output.completed_block().header(),
        tx,
        slot,
        epoch,
    );

    assert!(
        result.is_err(),
        "Update with invalid ledger reference should fail"
    );

    match result.unwrap_err() {
        ExecError::Acct(AcctError::InvalidLedgerReference { ref_idx, .. }) => {
            assert_eq!(
                ref_idx, manifest1_height,
                "Should fail on the invalid reference"
            );
        }
        err => panic!("Expected InvalidLedgerReference, got: {err:?}"),
    }

    // Verify no state change
    let snark_account = state.get_account_state(snark_id).unwrap().unwrap();
    assert_eq!(
        snark_account.balance(),
        BitcoinAmount::from_sat(100_000_000),
        "Balance should be unchanged after failed update"
    );
}
