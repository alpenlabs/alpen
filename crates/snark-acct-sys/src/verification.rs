use ssz::Encode as _;
use strata_acct_types::{AccountId, AcctError, MessageEntry};
use strata_ol_state_types::{ExecResult, ISnarkAccountState, TxProofVerifier};
use strata_snark_acct_types::*;
use tracing::warn;

use crate::update::{SnarkAccountUpdateData, effects_to_update_outputs};

/// Verifies an account update is correct with respect to the current state of
/// snark account, including checking account balances.
pub fn verify_update_correctness(
    target: AccountId,
    snark_state: &impl ISnarkAccountState,
    update: &SnarkAccountUpdateData,
    proof_verifier: &mut impl TxProofVerifier,
) -> ExecResult<()> {
    // 1. Check seq_no matches.
    verify_seq_no(target, snark_state, update.seq_no())?;

    // 2. Check message / proof entries and indices line up.
    verify_message_index(target, snark_state, update)?;

    // 3. Verify ledger references using the proof verifier.
    verify_ledger_refs(target, proof_verifier, update.ledger_refs())?;

    // 4. Verify inbox mmr proofs.
    verify_inbox_mmr_proofs(
        target,
        snark_state,
        proof_verifier,
        update.processed_messages(),
    )?;

    // 5. Verify the proof.
    verify_update_proof(target, snark_state, update, proof_verifier)?;

    Ok(())
}

/// Validates the update sequence number against the snark state.
pub fn verify_seq_no(
    target: AccountId,
    snark_state: &impl ISnarkAccountState,
    tx_seq_no: Seqno,
) -> ExecResult<()> {
    let expected_seq = snark_state.seqno();
    if *tx_seq_no.inner() != *expected_seq.inner() {
        return Err(AcctError::InvalidUpdateSequence {
            account_id: target,
            expected: *expected_seq.inner(),
            got: *tx_seq_no.inner(),
        }
        .into());
    }
    Ok(())
}

/// Validates the update message index against the snark state.
pub fn verify_message_index(
    target: AccountId,
    snark_state: &impl ISnarkAccountState,
    update: &SnarkAccountUpdateData,
) -> ExecResult<()> {
    let expected_idx = snark_state
        .next_inbox_msg_idx()
        .checked_add(update.processed_messages().len() as u64)
        .ok_or(AcctError::MsgIndexOverflow { account_id: target })?;

    let claimed_idx = update.new_proof_state().next_inbox_msg_idx();

    if expected_idx != claimed_idx {
        return Err(AcctError::InvalidMsgIndex {
            account_id: target,
            expected: expected_idx,
            got: claimed_idx,
        }
        .into());
    }

    Ok(())
}

/// Verifies ledger ref proofs using the OL L1 block refs accumulator.
fn verify_ledger_refs(
    target: AccountId,
    proof_verifier: &mut impl TxProofVerifier,
    ledger_refs: &LedgerRefs,
) -> ExecResult<()> {
    let l1_block_ref_claims = ledger_refs.l1_block_refs();

    for claim in l1_block_ref_claims {
        proof_verifier
            .verify_l1_block_ref_mmr_proof_next(claim)
            .map_err(|_| AcctError::InvalidLedgerReference {
                account_id: target,
                ref_idx: claim.idx(),
            })?;
    }

    Ok(())
}

/// Verifies the processed messages proofs against the account's inbox MMR
/// using the proof verifier.
fn verify_inbox_mmr_proofs(
    target: AccountId,
    state: &impl ISnarkAccountState,
    proof_verifier: &mut impl TxProofVerifier,
    processed_msgs: &[MessageEntry],
) -> ExecResult<()> {
    let mut cur_index = state.next_inbox_msg_idx();

    for msg in processed_msgs {
        let msg_hash = msg.compute_msg_commitment();
        let claim = AccumulatorClaim::new(cur_index, msg_hash);

        proof_verifier
            .verify_inbox_mmr_proof_next(&claim)
            .map_err(|_| AcctError::InvalidMessageProof {
                account_id: target,
                msg_idx: cur_index,
            })?;

        cur_index = cur_index
            .checked_add(1)
            .ok_or(AcctError::MsgIndexOverflow { account_id: target })?;
    }

    Ok(())
}

/// Verifies the update witness (proof and pub params) against the VK of the snark account.
pub(crate) fn verify_update_proof(
    target: AccountId,
    snark_state: &impl ISnarkAccountState,
    update: &SnarkAccountUpdateData,
    verifier: &mut impl TxProofVerifier,
) -> ExecResult<()> {
    let claim: Vec<u8> = compute_update_claim(snark_state, update);
    if let Err(e) = verifier.verify_local_predicate_next(&claim) {
        warn!(
            error = %e,
            account_id = %target,
            claim_len = claim.len(),
            "snark-account update proof rejected"
        );
        return Err(AcctError::InvalidUpdateProof { account_id: target }.into());
    }

    Ok(())
}

/// Computes the verifiable claim to be verified against a VK.
///
/// Converts [`TxEffects`] to [`UpdateOutputs`] for proof parameter construction.
fn compute_update_claim(
    snark_state: &impl ISnarkAccountState,
    update: &SnarkAccountUpdateData,
) -> Vec<u8> {
    let cur_state = ProofState::new(
        snark_state.inner_state_root(),
        snark_state.next_inbox_msg_idx(),
    );

    let outputs = effects_to_update_outputs(update.effects(), update.new_predicate());

    let pub_params = UpdateProofPubParams::new(
        update.seq_no(),
        cur_state,
        update.new_proof_state().clone(),
        update.processed_messages().to_vec(),
        update.ledger_refs().clone(),
        outputs,
        update.extra_data().to_vec(),
    );
    pub_params.as_ssz_bytes()
}

#[cfg(test)]
mod tests {
    use strata_acct_types::{AccumulatorClaim, Hash, TxEffects};
    use strata_ol_state_types::{ExecError, ProofVerifyError};
    use strata_ol_state_types_v1::OLSnarkAccountStateV1;
    use strata_predicate::PredicateKey;

    use super::*;

    struct ClaimVerifier {
        accepted_claim: Vec<u8>,
    }

    impl TxProofVerifier for ClaimVerifier {
        fn verify_inbox_mmr_proof_next(
            &mut self,
            _claim: &AccumulatorClaim,
        ) -> Result<(), ProofVerifyError> {
            unreachable!("the test update has no inbox proofs")
        }

        fn verify_l1_block_ref_mmr_proof_next(
            &mut self,
            _claim: &AccumulatorClaim,
        ) -> Result<(), ProofVerifyError> {
            unreachable!("the test update has no L1 block reference proofs")
        }

        fn verify_local_predicate_next(&mut self, claim: &[u8]) -> Result<(), ProofVerifyError> {
            if claim == self.accepted_claim {
                Ok(())
            } else {
                Err(ProofVerifyError::InvalidProof)
            }
        }

        fn is_exhausted(&self) -> bool {
            true
        }
    }

    #[test]
    fn rejects_proof_bound_to_unrecorded_public_pre_state() {
        let target = AccountId::zero();
        let recorded_root = Hash::from([1; 32]);
        let forged_root = Hash::from([2; 32]);
        let recorded_state =
            OLSnarkAccountStateV1::new_fresh(PredicateKey::always_accept(), recorded_root);
        let forged_state =
            OLSnarkAccountStateV1::new_fresh(PredicateKey::always_accept(), forged_root);
        let update = SnarkAccountUpdateData::new(
            Seqno::zero(),
            ProofState::new(Hash::from([3; 32]), 0),
            Vec::new(),
            LedgerRefs::new_empty(),
            TxEffects::default(),
            Vec::new(),
            None,
        );

        // Model a proof whose public inputs use an attacker-chosen pre-state.
        // The verifier accepts only the claim committed to by that proof.
        let forged_claim = compute_update_claim(&forged_state, &update);
        let mut verifier = ClaimVerifier {
            accepted_claim: forged_claim,
        };

        let result = verify_update_correctness(target, &recorded_state, &update, &mut verifier);

        assert!(matches!(
            result,
            Err(ExecError::Acct(AcctError::InvalidUpdateProof { account_id }))
                if account_id == target
        ));
    }
}
