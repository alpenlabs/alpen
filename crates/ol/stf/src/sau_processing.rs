//! Snark account update processing.

use strata_acct_types::{AccountId, TxEffects};
use strata_ledger_types::{IAccountState, TxProofVerifier};
use strata_ol_chain_types_new::*;
use strata_snark_acct_sys::SnarkAccountUpdateData;
use strata_snark_acct_types::{LedgerRefs, ProofState, Seqno};

use crate::errors::*;

/// Verifies a snark account update's proofs.  This assumes the tx has been
/// taken apart since we're presenting specifically the data from the tx for the
/// internal components to do what they need.
///
/// This is split out from tx processing because we want to be able to call it
/// from other contexts and be able to richly reason about the ledger ref proofs
/// we verify.
pub fn verify_snark_acct_update_proofs(
    target: AccountId,
    account_state: &impl IAccountState,
    sau_op: &SauTxOperationData,
    effects: &TxEffects,
    proof_verifier: &mut impl TxProofVerifier,
) -> ExecResult<()> {
    // 1. Extract snark account specific state.
    let snark_acct_state = account_state
        .as_snark_account()
        .map_err(|_| ExecError::IncorrectTxTargetType)?;

    // 2. Assemble the internal snark account update data.
    let update_data = build_snark_acct_update_data(sau_op, effects);

    // 3. Actually call out to the verifier logic.
    strata_snark_acct_sys::verify_update_correctness(
        target,
        snark_acct_state,
        &update_data,
        proof_verifier,
    )?;

    Ok(())
}

fn build_snark_acct_update_data(
    op: &SauTxOperationData,
    effects: &TxEffects,
) -> SnarkAccountUpdateData {
    let upd = op.update();
    let proof_state = ProofState::new(
        upd.proof_state().inner_state_root(),
        upd.proof_state().new_next_msg_idx(),
    );
    let ledger_refs = convert_sau_ledger_refs(op.ledger_refs());
    let processed_messages: Vec<_> = op.messages_iter().cloned().collect();

    SnarkAccountUpdateData::new(
        Seqno::from(upd.seq_no()),
        proof_state,
        processed_messages,
        ledger_refs,
        effects.clone(),
        upd.extra_data().to_vec(),
    )
}

fn convert_sau_ledger_refs(sau_refs: &SauTxLedgerRefs) -> LedgerRefs {
    match sau_refs.asm_history_proofs() {
        Some(claim_list) => LedgerRefs::new(claim_list.claims().to_vec()),
        None => LedgerRefs::new_empty(),
    }
}
