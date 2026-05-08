//! Tests for ledger references (referencing ASM manifests)

use ssz_primitives::FixedBytes;
use strata_acct_types::{
    AcctError, AccumulatorClaim, BitcoinAmount, RawMerkleProof, tree_hash::TreeHash,
};
use strata_asm_common::AsmManifest;
use strata_ledger_types::IStateAccessor;

use crate::{errors::ExecError, test_utils::*};

const FIRST_REAL_MANIFEST_INDEX: u64 = 1;

fn execute_manifest_block_with_tracker(
    fixture: &mut OLStfFixture,
    manifest: AsmManifest,
) -> (AccumulatorClaim, RawMerkleProof) {
    let mut manifest_tracker = ManifestMmrTracker::new();

    let manifest_hash = <AsmManifest as TreeHash>::tree_hash_root(&manifest);

    fixture
        .child_block()
        .with_manifest(manifest.clone())
        .execute_with_outputs();

    let (manifest_index, manifest_proof) = manifest_tracker.add_manifest(&manifest);

    assert_eq!(
        fixture.state().asm_manifests_mmr().num_entries(),
        manifest_tracker.num_entries(),
        "State MMR should match tracker MMR"
    );
    assert_eq!(
        manifest_index, FIRST_REAL_MANIFEST_INDEX,
        "first real manifest lands after the genesis sentinel prefill"
    );

    (
        AccumulatorClaim::new(manifest_index, manifest_hash.into_inner()),
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

#[test]
fn test_snark_update_with_valid_ledger_reference() {
    let snark_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient_id = make_account_id(TEST_RECIPIENT_ID);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .with_genesis_empty_account(recipient_id)
        .execute_genesis();

    let (claim, proof) =
        execute_manifest_block_with_tracker(&mut fixture, make_empty_manifest(1, 1));

    fixture
        .child_block()
        .with_sau(snark_id, |sau| {
            sau.transfer(recipient_id, BitcoinAmount::from_sat(10_000_000))
                .with_ledger_refs(vec![claim], vec![proof])
                .with_state_root(make_state_root(2))
                .with_proof(vec![0u8; 32])
        })
        .execute();

    assert_eq!(
        fixture.account_balance(snark_id),
        BitcoinAmount::from_sat(90_000_000),
        "Sender balance should be reduced"
    );
    assert_eq!(
        fixture.account_balance(recipient_id),
        BitcoinAmount::from_sat(10_000_000),
        "Recipient should receive transfer"
    );
}

#[test]
fn test_snark_update_with_invalid_ledger_reference() {
    let snark_id = make_account_id(TEST_SNARK_ACCOUNT_ID);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .execute_genesis();

    let (claim, proof) =
        execute_manifest_block_with_tracker(&mut fixture, make_empty_manifest(1, 1));
    let invalid_proof = corrupt_proof(proof);

    let err = fixture
        .child_block()
        .with_sau(snark_id, |sau| {
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

    assert_eq!(
        fixture.account_balance(snark_id),
        BitcoinAmount::from_sat(100_000_000),
        "Balance should be unchanged after failed update"
    );
}

#[test]
fn test_snark_update_with_mismatched_ledger_reference_proof_index() {
    let snark_id = make_account_id(TEST_SNARK_ACCOUNT_ID);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .execute_genesis();

    let (claim, proof) =
        execute_manifest_block_with_tracker(&mut fixture, make_empty_manifest(1, 1));
    let mismatched_proof = corrupt_proof(proof);

    let err = fixture
        .child_block()
        .with_sau(snark_id, |sau| {
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
}
