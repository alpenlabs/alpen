use ssz::Encode as _;
use strata_acct_types::{AccountId, AcctError, AcctResult, MessageEntry};
use strata_identifiers::L1Height;
use strata_ledger_types::{
    ISnarkAccountState, IStateAccessor, TxProofVerifier, asm_manifest_mmr_index_for_height,
};
use strata_snark_acct_types::*;

use crate::update::{SnarkAccountUpdateData, effects_to_update_outputs};

/// Verifies an account update is correct with respect to the current state of
/// snark account, including checking account balances.
pub fn verify_update_correctness<S: IStateAccessor>(
    state_accessor: &S,
    target: AccountId,
    snark_state: &impl ISnarkAccountState,
    update: &SnarkAccountUpdateData,
    proof_verifier: &mut impl TxProofVerifier,
) -> AcctResult<()> {
    // 1. Check seq_no matches.
    verify_seq_no(target, snark_state, update.seq_no())?;

    // 2. Check message / proof entries and indices line up.
    verify_message_index(target, snark_state, update)?;

    // 3. Verify ledger references using the proof verifier.
    verify_ledger_refs(target, state_accessor, proof_verifier, update.ledger_refs())?;

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
) -> AcctResult<()> {
    let expected_seq = snark_state.seqno();
    if *tx_seq_no.inner() != *expected_seq.inner() {
        return Err(AcctError::InvalidUpdateSequence {
            account_id: target,
            expected: *expected_seq.inner(),
            got: *tx_seq_no.inner(),
        });
    }
    Ok(())
}

/// Validates the update message index against the snark state.
pub fn verify_message_index(
    target: AccountId,
    snark_state: &impl ISnarkAccountState,
    update: &SnarkAccountUpdateData,
) -> AcctResult<()> {
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
        });
    }

    Ok(())
}

/// Verifies the ledger ref proofs against the ASM manifest MMR using the proof verifier.
///
/// For each ledger reference, resolves the L1 height to an MMR index, constructs
/// an [`AccumulatorClaim`], and delegates verification to the proof verifier.
fn verify_ledger_refs(
    target: AccountId,
    state_accessor: &impl IStateAccessor,
    proof_verifier: &mut impl TxProofVerifier,
    ledger_refs: &LedgerRefs,
) -> AcctResult<()> {
    let manifest_refs = ledger_refs.l1_header_refs();

    for manifest_ref in manifest_refs {
        let l1_height: L1Height =
            manifest_ref
                .idx()
                .try_into()
                .map_err(|_| AcctError::InvalidLedgerReference {
                    account_id: target,
                    ref_idx: manifest_ref.idx(),
                })?;

        let mmr_idx =
            asm_manifest_mmr_index_for_height(state_accessor, l1_height).ok_or_else(|| {
                AcctError::InvalidLedgerReference {
                    account_id: target,
                    ref_idx: manifest_ref.idx(),
                }
            })?;

        let claim = AccumulatorClaim::new(mmr_idx, manifest_ref.entry_hash());
        proof_verifier
            .verify_asm_history_mmr_proof_next(&claim)
            .map_err(|_| AcctError::InvalidLedgerReference {
                account_id: target,
                ref_idx: manifest_ref.idx(),
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
) -> AcctResult<()> {
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
) -> AcctResult<()> {
    let claim: Vec<u8> = compute_update_claim(snark_state, update);
    let is_valid = verifier.verify_local_predicate_next(&claim).is_ok();

    if !is_valid {
        return Err(AcctError::InvalidUpdateProof { account_id: target });
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

    let outputs = effects_to_update_outputs(update.effects());

    let pub_params = UpdateProofPubParams::new(
        cur_state,
        update.new_proof_state().clone(),
        update.processed_messages().to_vec(),
        update.ledger_refs().clone(),
        outputs,
        update.extra_data().to_vec(),
    );
    pub_params.as_ssz_bytes()
}
