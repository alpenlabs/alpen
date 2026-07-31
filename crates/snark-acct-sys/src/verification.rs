use ssz::Encode as _;
use strata_acct_types::{AccountId, AcctError, MessageEntry};
use strata_ledger_types::{ExecResult, ISnarkAccountState, TxProofVerifier};
use strata_snark_acct_types::*;
use tracing::warn;

use crate::update::effects_to_update_outputs;

/// Verifies an account update is correct with respect to the current state of
/// snark account, including checking account balances.
pub fn verify_update_correctness(
    target: AccountId,
    snark_state: &impl ISnarkAccountState,
    update: impl ISnarkAccountUpdateData,
    proof_verifier: &mut impl TxProofVerifier,
) -> ExecResult<()> {
    // 1. Check seq_no matches.
    verify_seq_no(target, snark_state, update.seq_no())?;

    // 2. Check message / proof entries and indices line up.
    verify_message_index(target, snark_state, &update)?;

    // 3. Verify ledger references using the proof verifier.
    verify_ledger_refs(target, proof_verifier, update.ledger_refs())?;

    // 4. Verify inbox mmr proofs.
    verify_inbox_mmr_proofs(target, snark_state, proof_verifier, &update)?;

    // 5. Verify the proof.
    verify_update_proof(target, snark_state, &update, proof_verifier)?;

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
    update: &impl ISnarkAccountUpdateData,
) -> ExecResult<()> {
    let expected_idx = snark_state
        .next_inbox_msg_idx()
        .checked_add(update.num_messasges())
        .ok_or(AcctError::MsgIndexOverflow { account_id: target })?;

    let claimed_idx = update.new_next_msg_idx();

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
    ledger_refs: impl ILedgerRefs,
) -> ExecResult<()> {
    for claim in ledger_refs.l1_block_refs_iter() {
        proof_verifier
            .verify_l1_block_ref_mmr_proof_next(&claim)
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
    update: &impl ISnarkAccountUpdateData,
) -> ExecResult<()> {
    let mut cur_index = state.next_inbox_msg_idx();

    for msg in update.messages_iter() {
        let msg_hash = msg.compute_commitment();
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
    update: &impl ISnarkAccountUpdateData,
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
    update: &impl ISnarkAccountUpdateData,
) -> Vec<u8> {
    let cur_state = ProofState::new(
        snark_state.inner_state_root(),
        snark_state.next_inbox_msg_idx(),
    );

    let outputs = effects_to_update_outputs(update.effects());

    let pub_params = UpdateProofPubParams::new(
        update.seq_no(),
        cur_state,
        ProofState::new(update.new_inner_state(), update.new_next_msg_idx()),
        update
            .messages_iter()
            .map(|e| MessageEntry::new(e.source(), e.incl_epoch(), e.get_payload()))
            .collect::<Vec<_>>(),
        LedgerRefs::new(update.ledger_refs().l1_block_refs_iter().collect()),
        outputs,
        update.extra_data().to_vec(),
    );
    pub_params.as_ssz_bytes()
}
