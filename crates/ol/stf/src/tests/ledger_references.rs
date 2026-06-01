//! Tests for ledger references (referencing ASM manifests)

use ssz_primitives::FixedBytes;
use strata_acct_types::{
    AccountId, AcctError, AccumulatorClaim, BitcoinAmount, RawMerkleProof,
    l1_block_record_leaf_hash,
};
use strata_asm_common::AsmManifest;
use strata_ledger_types::{ISnarkAccountState, IStateAccessor};
use strata_ol_chain_types_new::{
    OLTransaction, ProofSatisfier, ProofSatisfierList, RawMerkleProofList, TxProofs,
};

use crate::{errors::ExecError, test_utils::*};

// The test genesis state pre-fills manifest MMR index 0 with
// `strata_ol_state_types::MMR_SENTINEL_DUMMY_LEAF`; real manifests start at 1.
const FIRST_REAL_MANIFEST_INDEX: u64 = 1;

fn execute_manifest_block_with_tracker(
    fixture: &mut OLStfFixture,
    manifest: AsmManifest,
) -> (AccumulatorClaim, RawMerkleProof) {
    let mut manifest_tracker = ManifestMmrTracker::new();

    let l1_block_ref_hash =
        l1_block_record_leaf_hash(manifest.blkid().as_ref(), manifest.wtxids_root().as_ref());

    fixture
        .child_block()
        .with_manifest(manifest.clone())
        .execute_with_outputs();

    let (manifest_index, manifest_proof) = manifest_tracker.add_manifest(&manifest);

    assert_eq!(
        fixture.state().l1_block_refs_mmr().num_entries(),
        manifest_tracker.num_entries(),
        "State MMR should match tracker MMR"
    );
    assert_eq!(
        manifest_index, FIRST_REAL_MANIFEST_INDEX,
        "first real manifest lands after the genesis sentinel prefill"
    );

    (
        AccumulatorClaim::new(manifest_index, l1_block_ref_hash),
        manifest_proof,
    )
}

fn corrupt_proof(proof: RawMerkleProof) -> RawMerkleProof {
    let mut cohashes: Vec<_> = proof.cohashes.iter().cloned().collect();
    if cohashes.is_empty() {
        cohashes.push(FixedBytes::<32>::from([0xee; 32]));
    } else {
        cohashes[0] = FixedBytes::<32>::from([0xee; 32]);
    }

    RawMerkleProof {
        cohashes: cohashes
            .try_into()
            .expect("proof should not exceed capacity"),
    }
}

fn expect_invalid_update_proof(err: ExecError, account_id: AccountId) {
    match err.into_base() {
        ExecError::Acct(AcctError::InvalidUpdateProof { account_id: actual }) => {
            assert_eq!(actual, account_id);
        }
        err => panic!("Expected InvalidUpdateProof, got: {err:?}"),
    }
}

#[test]
fn test_snark_update_with_valid_ledger_reference() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient_id = make_account_id(TEST_RECIPIENT_ID);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .with_genesis_empty_account(recipient_id)
        .execute_genesis();

    let (claim, proof) =
        execute_manifest_block_with_tracker(&mut fixture, make_empty_manifest(1, 1));

    fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            sau.transfer(recipient_id, BitcoinAmount::from_sat(10_000_000))
                .with_ledger_refs(vec![claim], vec![proof])
                .with_state_root(make_state_root(2))
                .with_proof(vec![0u8; 32])
        })
        .execute();

    assert_eq!(
        fixture.account_balance(snark_acct_id),
        BitcoinAmount::from_sat(90_000_000),
        "Sender balance should be reduced"
    );
    assert_eq!(
        *fixture.expect_snark_account(snark_acct_id).seqno().inner(),
        1,
        "Sender account seq no should increase"
    );
    assert_eq!(
        fixture.account_balance(recipient_id),
        BitcoinAmount::from_sat(10_000_000),
        "Recipient should receive transfer"
    );
}

#[test]
fn test_snark_update_with_invalid_ledger_reference() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .execute_genesis();

    let (claim, proof) =
        execute_manifest_block_with_tracker(&mut fixture, make_empty_manifest(1, 1));
    let invalid_proof = corrupt_proof(proof);
    let snapshot = fixture.snapshot([snark_acct_id]);

    let err = fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            sau.with_ledger_refs(vec![claim], vec![invalid_proof])
                .with_state_root(make_state_root(2))
                .with_proof(vec![0u8; 32])
        })
        .execute_err();

    match err.into_base() {
        ExecError::Acct(AcctError::InvalidLedgerReference { ref_idx, .. }) => {
            assert_eq!(
                ref_idx, FIRST_REAL_MANIFEST_INDEX,
                "Should fail on the invalid reference"
            );
        }
        err => panic!("Expected InvalidLedgerReference, got: {err:?}"),
    }

    snapshot.assert_unchanged(&fixture);
}

#[test]
fn test_snark_update_with_mismatched_ledger_reference_proof_index() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .execute_genesis();

    let (claim, proof) =
        execute_manifest_block_with_tracker(&mut fixture, make_empty_manifest(1, 1));
    let mismatched_proof = corrupt_proof(proof);
    let snapshot = fixture.snapshot([snark_acct_id]);

    let err = fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            sau.with_ledger_refs(vec![claim], vec![mismatched_proof])
                .with_state_root(make_state_root(2))
                .with_proof(vec![0u8; 32])
        })
        .execute_err();

    match err.into_base() {
        ExecError::Acct(AcctError::InvalidLedgerReference { ref_idx, .. }) => {
            assert_eq!(ref_idx, FIRST_REAL_MANIFEST_INDEX);
        }
        err => panic!("Expected InvalidLedgerReference, got: {err:?}"),
    }

    snapshot.assert_unchanged(&fixture);
}

#[test]
fn test_snark_update_rejects_proof_for_wrong_ledger_reference_claim() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .execute_genesis();
    let mut manifest_tracker = ManifestMmrTracker::new();

    let manifest1 = make_empty_manifest(1, 1);
    let manifest1_hash =
        l1_block_record_leaf_hash(manifest1.blkid().as_ref(), manifest1.wtxids_root().as_ref());
    let manifest2 = make_empty_manifest(2, 2);

    fixture
        .child_block()
        .with_manifest(manifest1.clone())
        .with_manifest(manifest2.clone())
        .execute_with_outputs();

    let (manifest1_index, _manifest1_proof) = manifest_tracker.add_manifest(&manifest1);
    let (manifest2_index, manifest2_proof) = manifest_tracker.add_manifest(&manifest2);

    assert_eq!(
        manifest1_index, FIRST_REAL_MANIFEST_INDEX,
        "First real manifest should follow the genesis sentinel"
    );
    assert_eq!(
        manifest2_index,
        FIRST_REAL_MANIFEST_INDEX + 1,
        "Second real manifest should follow the first real manifest"
    );
    assert_eq!(
        fixture.state().l1_block_refs_mmr().num_entries(),
        manifest_tracker.num_entries(),
        "State MMR should match tracker MMR"
    );

    let claim = AccumulatorClaim::new(manifest1_index, manifest1_hash);
    let initial_balance = fixture.account_balance(snark_acct_id);
    let initial_seqno = *fixture.expect_snark_account(snark_acct_id).seqno().inner();

    let err = fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            sau.with_ledger_refs(vec![claim], vec![manifest2_proof])
                .with_state_root(make_state_root(2))
                .with_proof(vec![0u8; 32])
        })
        .execute_err();

    match err.into_base() {
        ExecError::Acct(AcctError::InvalidLedgerReference { ref_idx, .. }) => {
            assert_eq!(ref_idx, manifest1_index);
        }
        err => panic!("Expected InvalidLedgerReference, got: {err:?}"),
    }

    assert_eq!(
        fixture.account_balance(snark_acct_id),
        initial_balance,
        "balance should be unchanged after proof for the wrong ledger reference claim"
    );
    assert_eq!(
        *fixture.expect_snark_account(snark_acct_id).seqno().inner(),
        initial_seqno,
        "seqno should be unchanged after proof for the wrong ledger reference claim"
    );
}

#[test]
fn test_snark_update_rejects_extra_ledger_reference_proof() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .execute_genesis();

    let (claim, proof) =
        execute_manifest_block_with_tracker(&mut fixture, make_empty_manifest(1, 1));
    let extra_proof = proof.clone();

    let tx =
        SnarkUpdateBuilder::from_snark_state(fixture.expect_snark_account(snark_acct_id).clone())
            .with_ledger_refs(vec![claim], vec![proof, extra_proof])
            .build(snark_acct_id, make_state_root(2), vec![0u8; 32]);

    let err = fixture.child_block().with_tx(tx).execute_err();
    expect_invalid_update_proof(err, snark_acct_id);
}

#[test]
fn test_snark_update_rejects_accumulator_proof_without_ledger_refs() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .execute_genesis();

    let tx =
        SnarkUpdateBuilder::from_snark_state(fixture.expect_snark_account(snark_acct_id).clone())
            .build(snark_acct_id, make_state_root(2), vec![0u8; 32])
            .with_accumulator_proofs(Some(
                RawMerkleProofList::from_vec_nonempty(vec![RawMerkleProof::new_zero()])
                    .expect("non-empty proof list should be valid"),
            ));

    let err = fixture.child_block().with_tx(tx).execute_err();
    expect_invalid_update_proof(err, snark_acct_id);
}

#[test]
fn test_snark_update_rejects_extra_predicate_satisfier() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .execute_genesis();

    let base_tx =
        SnarkUpdateBuilder::from_snark_state(fixture.expect_snark_account(snark_acct_id).clone())
            .build(snark_acct_id, make_state_root(3), make_proof(1));
    let pred1 = ProofSatisfier::from_vec(make_proof(1)).expect("predicate proof should fit");
    let pred2 = ProofSatisfier::from_vec(make_proof(2)).expect("predicate proof should fit");
    let predicate_satisfiers = ProofSatisfierList::from_proofs(vec![pred1, pred2])
        .expect("predicate satisfier list should fit");
    let tx = OLTransaction::new(
        base_tx.data().clone(),
        TxProofs::new(
            Some(predicate_satisfiers),
            base_tx.proofs().accumulator_proofs().cloned(),
        ),
    );

    let err = fixture.child_block().with_tx(tx).execute_err();
    expect_invalid_update_proof(err, snark_acct_id);
}
