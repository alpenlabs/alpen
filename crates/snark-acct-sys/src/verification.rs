use ssz::Encode as _;
use strata_acct_types::{
    AccountId, AcctError, AcctResult, BitcoinAmount, MessageEntry, tree_hash::TreeHash,
};
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
    cur_balance: BitcoinAmount,
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

    // 5. Verify outputs can be applied safely.
    verify_effects_safe(update, state_accessor, cur_balance)?;

    // 6. Verify the proof.
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
        let msg_hash = <MessageEntry as TreeHash>::tree_hash_root(msg).into_inner();
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

/// Verifies that the effects in the update are safe (recipients exist, balance sufficient).
fn verify_effects_safe<S: IStateAccessor>(
    update: &SnarkAccountUpdateData,
    state_accessor: &S,
    cur_balance: BitcoinAmount,
) -> AcctResult<()> {
    let effects = update.effects();

    // Check if receivers exist (skip special/system accounts).
    for t in effects.transfers_iter() {
        if !t.dest().is_special() && !state_accessor.check_account_exists(t.dest())? {
            return Err(AcctError::MissingExpectedAccount(t.dest()));
        }
    }

    for m in effects.messages_iter() {
        if !m.dest().is_special() && !state_accessor.check_account_exists(m.dest())? {
            return Err(AcctError::MissingExpectedAccount(m.dest()));
        }
    }

    let total_sent =
        compute_effects_total_value(effects).ok_or(AcctError::BitcoinAmountOverflow)?;

    // Check if there is sufficient balance.
    if total_sent > cur_balance {
        return Err(AcctError::InsufficientBalance {
            requested: total_sent,
            available: cur_balance,
        });
    }

    Ok(())
}

/// Computes the total value of all transfers and messages in effects.
fn compute_effects_total_value(effects: &strata_acct_types::TxEffects) -> Option<BitcoinAmount> {
    let mut total: u64 = 0;

    for t in effects.transfers_iter() {
        total = total.checked_add(t.value().into())?;
    }

    for m in effects.messages_iter() {
        total = total.checked_add(m.payload().value().into())?;
    }

    Some(BitcoinAmount::from_sat(total))
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
