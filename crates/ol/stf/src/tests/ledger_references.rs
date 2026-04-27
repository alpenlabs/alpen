//! Tests for ledger references (referencing ASM manifests)

use strata_acct_types::{
    AcctError, AccumulatorClaim, BitcoinAmount, RawMerkleProof, tree_hash::TreeHash,
};
use strata_asm_common::AsmManifest;
use strata_identifiers::{Buf32, WtxidsRoot};
use strata_ledger_types::{IAccountState, IStateAccessor};
use strata_ol_chain_types_new::{
    OLTransaction, ProofSatisfier, ProofSatisfierList, RawMerkleProofList, TxProofs,
};

use crate::{assembly::BlockComponents, context::BlockInfo, errors::ExecError, test_utils::*};

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
    )
    .expect("test manifest should be valid");

    // Get the manifest hash before execution
    let manifest1_hash = <AsmManifest as TreeHash>::tree_hash_root(&manifest1);

    // Execute block with manifest
    let block1_info = BlockInfo::new(1001000, 1, 1); // slot 1, epoch 1
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
    // AccumulatorClaim.idx is the MMR leaf index directly
    let claim = AccumulatorClaim::new(manifest1_index, manifest1_hash.into_inner());

    // Create update with ledger reference and a transfer using SnarkUpdateBuilder
    let snark_account_state = lookup_snark_state(&state, snark_id);

    let tx = SnarkUpdateBuilder::from_snark_state(snark_account_state.clone())
        .with_transfer(recipient_id, 10_000_000)
        .with_ledger_refs(vec![claim], vec![manifest1_proof])
        .build(snark_id, get_test_state_root(2), vec![0u8; 32]);

    // Step 3: Execute the update
    let (slot, epoch) = (2, 2); // Increment epoch because genesis and block 1 are terminal
    execute_tx_in_block(
        &mut state,
        block1_output.completed_block().header(),
        tx,
        slot,
        epoch,
    )
    .expect("Update with valid ledger reference should succeed");

    // Verify the transfer was applied
    let (ol_account_state, _) = lookup_snark_account_states(&state, snark_id);
    assert_eq!(
        ol_account_state.balance(),
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
    )
    .expect("test manifest should be valid");

    // Get the manifest hash before execution
    let manifest1_hash = <AsmManifest as TreeHash>::tree_hash_root(&manifest1);

    // Execute block with manifest
    let block1_info = BlockInfo::new(1001000, 1, 1); // slot 1, epoch 1
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
    // AccumulatorClaim.idx is the MMR leaf index directly
    let claim = AccumulatorClaim::new(manifest1_index, manifest1_hash.into_inner());

    // Create an invalid proof with wrong cohashes
    let invalid_proof = RawMerkleProof {
        cohashes: vec![ssz_primitives::FixedBytes::<32>::from([0xff; 32])]
            .try_into()
            .unwrap(),
    };

    // Create update with invalid ledger reference using SnarkUpdateBuilder
    let snark_account_state = lookup_snark_state(&state, snark_id);

    let tx = SnarkUpdateBuilder::from_snark_state(snark_account_state.clone())
        .with_ledger_refs(vec![claim], vec![invalid_proof])
        .build(snark_id, get_test_state_root(2), vec![0u8; 32]);

    // Step 3: Execute and expect failure
    let (slot, epoch) = (2, 2); // Increment epoch because genesis and block 1 are terminal
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

    match result.unwrap_err().into_base() {
        ExecError::Acct(AcctError::InvalidLedgerReference { ref_idx, .. }) => {
            assert_eq!(
                ref_idx, manifest1_index,
                "Should fail on the invalid reference"
            );
        }
        err => panic!("Expected InvalidLedgerReference, got: {err:?}"),
    }

    // Verify no state change
    let (ol_account_state, _) = lookup_snark_account_states(&state, snark_id);
    assert_eq!(
        ol_account_state.balance(),
        BitcoinAmount::from_sat(100_000_000),
        "Balance should be unchanged after failed update"
    );
}

#[test]
fn test_snark_update_with_mismatched_ledger_reference_proof_index() {
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
    )
    .expect("test manifest should be valid");
    let manifest1_hash = <AsmManifest as TreeHash>::tree_hash_root(&manifest1);

    let block1_info = BlockInfo::new(1001000, 1, 1); // slot 1, epoch 1
    let block1_components = BlockComponents::new_manifests(vec![manifest1.clone()]);
    let block1_output = execute_block_with_outputs(
        &mut state,
        &block1_info,
        Some(genesis_block.header()),
        block1_components,
    )
    .expect("Block 1 should execute");

    let (manifest1_index, manifest1_proof) = manifest_tracker.add_manifest(&manifest1);

    // Step 2: Create a reference claim with a proof that carries a wrong entry index.
    let claim = AccumulatorClaim::new(manifest1_index, manifest1_hash.into_inner());

    // Create a mismatched proof by using the valid cohashes but a wrong index
    // We reconstruct the proof with wrong index via RawMerkleProof (which strips the index)
    // but shift the cohashes to produce a wrong root.
    // Actually we need to create a RawMerkleProof with wrong cohash content to mismatch.
    // The simplest way: just reverse the cohashes from the valid proof.
    let mismatched_proof = RawMerkleProof {
        cohashes: {
            let mut cohashes: Vec<_> = manifest1_proof.cohashes.iter().cloned().collect();
            // Corrupt the proof by replacing the first cohash with a bogus one
            if !cohashes.is_empty() {
                cohashes[0] = ssz_primitives::FixedBytes::<32>::from([0xff; 32]);
            } else {
                // If no cohashes, add a bogus one
                cohashes.push(ssz_primitives::FixedBytes::<32>::from([0xff; 32]));
            }
            cohashes
                .try_into()
                .expect("Proof should not exceed capacity")
        },
    };

    let snark_account_state = lookup_snark_state(&state, snark_id);

    let tx = SnarkUpdateBuilder::from_snark_state(snark_account_state.clone())
        .with_ledger_refs(vec![claim], vec![mismatched_proof])
        .build(snark_id, get_test_state_root(2), vec![0u8; 32]);

    // Step 3: Execute and expect failure due to proof index mismatch.
    let result = execute_tx_in_block(
        &mut state,
        block1_output.completed_block().header(),
        tx,
        2,
        2,
    );

    match result {
        Err(e) => match e.into_base() {
            ExecError::Acct(AcctError::InvalidLedgerReference { ref_idx, .. }) => {
                assert_eq!(ref_idx, manifest1_index);
            }
            err => panic!("Expected InvalidLedgerReference, got: {err:?}"),
        },
        Ok(_) => panic!("Update with mismatched proof index should fail"),
    }
}

#[test]
fn test_snark_update_rejects_extra_ledger_reference_proof() {
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
    )
    .expect("test manifest should be valid");
    let manifest1_hash = <AsmManifest as TreeHash>::tree_hash_root(&manifest1);

    let block1_info = BlockInfo::new(1001000, 1, 1); // slot 1, epoch 1
    let block1_components = BlockComponents::new_manifests(vec![manifest1.clone()]);
    let block1_output = execute_block_with_outputs(
        &mut state,
        &block1_info,
        Some(genesis_block.header()),
        block1_components,
    )
    .expect("Block 1 should execute");

    let (manifest1_index, manifest1_proof) = manifest_tracker.add_manifest(&manifest1);

    // Step 2: Build an update with one claim but two accumulator proofs.
    // The first proof is consumed by the claim; the second must be rejected.
    let claim = AccumulatorClaim::new(manifest1_index, manifest1_hash.into_inner());
    let extra_proof = manifest1_proof.clone();

    let snark_account_state = lookup_snark_state(&state, snark_id);

    let tx = SnarkUpdateBuilder::from_snark_state(snark_account_state.clone())
        .with_ledger_refs(vec![claim], vec![manifest1_proof, extra_proof])
        .build(snark_id, get_test_state_root(2), vec![0u8; 32]);

    // Step 3: Execute and expect failure due to unconsumed accumulator proof.
    let result = execute_tx_in_block(
        &mut state,
        block1_output.completed_block().header(),
        tx,
        2,
        2,
    );

    match result {
        Err(e) => match e.into_base() {
            ExecError::Acct(AcctError::InvalidUpdateProof { account_id }) => {
                assert_eq!(account_id, snark_id);
            }
            err => panic!("Expected InvalidUpdateProof, got: {err:?}"),
        },
        Ok(_) => panic!("Update with an extra ledger reference proof should fail"),
    }
}

#[test]
fn test_snark_update_rejects_accumulator_proof_without_ledger_refs() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();

    // Setup: genesis with snark account
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

    let snark_account_state = lookup_snark_state(&state, snark_id);

    // No ledger refs and no inbox messages to consume accumulator proofs.
    let tx = SnarkUpdateBuilder::from_snark_state(snark_account_state.clone())
        .build(snark_id, get_test_state_root(2), vec![0u8; 32])
        .with_accumulator_proofs(Some(
            RawMerkleProofList::from_vec_nonempty(vec![RawMerkleProof::new_zero()])
                .expect("non-empty proof list should be valid"),
        ));

    let result = execute_tx_in_block(&mut state, genesis_block.header(), tx, 1, 1);

    match result {
        Err(e) => match e.into_base() {
            ExecError::Acct(AcctError::InvalidUpdateProof { account_id }) => {
                assert_eq!(account_id, snark_id);
            }
            err => panic!("Expected InvalidUpdateProof, got: {err:?}"),
        },
        Ok(_) => panic!("Update with unconsumed accumulator proof should fail"),
    }
}

#[test]
fn test_snark_update_rejects_extra_predicate_satisfier() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();

    // Setup: genesis with snark account
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

    let snark_account_state = lookup_snark_state(&state, snark_id);

    // Build a valid update first.
    let base_tx = SnarkUpdateBuilder::from_snark_state(snark_account_state.clone()).build(
        snark_id,
        get_test_state_root(3),
        get_test_proof(1),
    );
    let pred1 = ProofSatisfier::from_vec(get_test_proof(1)).expect("predicate proof should fit");
    let pred2 = ProofSatisfier::from_vec(get_test_proof(2)).expect("predicate proof should fit");
    let predicate_satisfiers = ProofSatisfierList::from_proofs(vec![pred1, pred2])
        .expect("predicate satisfier list should fit");
    let tx = OLTransaction::new(
        base_tx.data().clone(),
        TxProofs::new(
            Some(predicate_satisfiers),
            base_tx.proofs().accumulator_proofs().cloned(),
        ),
    );

    let result = execute_tx_in_block(&mut state, genesis_block.header(), tx, 1, 1);

    match result {
        Err(e) => match e.into_base() {
            ExecError::Acct(AcctError::InvalidUpdateProof { account_id }) => {
                assert_eq!(account_id, snark_id);
            }
            err => panic!("Expected InvalidUpdateProof, got: {err:?}"),
        },
        Ok(_) => panic!("Update with extra predicate satisfier should fail"),
    }
}
