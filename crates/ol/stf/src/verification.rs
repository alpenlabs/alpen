use strata_acct_types::{AccountId, MerkleProof, StrataHasher};
use strata_ledger_types::{AccountTypeState, IAccountState, ISnarkAccountState, StateAccessor};
use strata_mmr::hasher::MerkleHasher;
use strata_snark_acct_types::{
    LedgerRefs, MessageEntry, SnarkAccountUpdate, UpdateOperationData, UpdateOutputs,
};

use crate::error::{StfError, StfResult};

/// Verifies an account update is correct with respect to the current state of
/// snark account, including checking account balances.
pub(crate) fn verify_update_correctness<'a, S: StateAccessor>(
    state_accessor: &S,
    sender: AccountId,
    acct_state: &S::AccountState,
    update: &'a SnarkAccountUpdate,
) -> StfResult<VerifiedUpdate<'a>> {
    let type_state = acct_state.get_type_state()?;
    let operation = update.operation();
    let outputs = operation.outputs();
    let state = match type_state {
        AccountTypeState::Empty => {
            return Ok(VerifiedUpdate { operation });
        }
        AccountTypeState::Snark(s) => s,
    };

    // 1. Check seq_no matches
    if state.seqno() != operation.seq_no() {
        return Err(StfError::InvalidUpdateSequence {
            account_id: sender,
            expected: state.seqno(),
            got: operation.seq_no(),
        });
    }

    // 2. Check message counts / proof indices line up
    let cur_idx = state.next_inbox_idx();
    let new_idx = operation.new_state().next_inbox_msg_idx();
    let processed_len = operation.processed_messages().len() as u64;

    let expected_idx = cur_idx
        .checked_add(processed_len)
        .ok_or(StfError::MsgIndexOverflow { account_id: sender })?;
    if expected_idx != new_idx {
        return Err(StfError::InvalidMsgIndex {
            account_id: sender,
            expected: expected_idx,
            got: new_idx,
        });
    }

    // 3. Verify ledger references using the provided state accessor
    verify_ledger_refs(operation.ledger_refs(), state_accessor)?;

    // Create message proofs
    let message_proofs = get_message_proofs::<S>(sender, update, &state)?;
    // 4. Verify input mmr proofs
    verify_input_mmr_proofs::<S>(sender, &state, &message_proofs)?;

    // 4. Verify outputs can be applied safely
    verify_update_outputs_safe(outputs, sender, acct_state, state_accessor)?;

    // 5. Verify the witness check
    verify_update_witness(sender, acct_state, update, state_accessor)?;

    Ok(VerifiedUpdate { operation })
}

fn get_message_proofs<S: StateAccessor>(
    sender: AccountId,
    update: &SnarkAccountUpdate,
    state: &<S::AccountState as IAccountState>::SnarkAccountState,
) -> StfResult<Vec<(MessageEntry, MerkleProof)>> {
    let mut cur_idx = state.next_inbox_idx();
    let mut proofs = Vec::new();

    for msg in update.operation().processed_messages() {
        let proof =
            state
                .get_message_proof(msg, cur_idx)?
                .ok_or(StfError::InvalidMessageProof {
                    account_id: sender,
                    message: msg.clone(),
                })?;

        let mproof = MerkleProof::from_cohashes(proof.raw_proof().cohashes().to_vec(), cur_idx);
        proofs.push((msg.clone(), mproof));
        cur_idx = cur_idx
            .checked_add(1)
            .ok_or(StfError::MsgIndexOverflow { account_id: sender })?;
    }
    Ok(proofs)
}

pub(crate) fn verify_input_mmr_proofs<S: StateAccessor>(
    account_id: AccountId,
    state: &<S::AccountState as IAccountState>::SnarkAccountState,
    msg_proofs: &[(MessageEntry, MerkleProof)],
) -> StfResult<()> {
    for (msg, proof) in msg_proofs {
        let msg_bytes: Vec<u8> = msg.as_ssz_bytes();
        let hash = StrataHasher::hash_leaf(&msg_bytes);
        if !state.inbox_mmr().verify(proof, &hash) {
            return Err(StfError::InvalidMessageProof {
                account_id,
                message: msg.clone(),
            });
        }
    }
    Ok(())
}

pub(crate) fn verify_update_witness<S: StateAccessor>(
    sender: AccountId,
    acct_state: &<S as StateAccessor>::AccountState,
    update: &SnarkAccountUpdate,
    _state_accessor: &S,
) -> StfResult<()> {
    let snark_state = match acct_state.get_type_state()? {
        AccountTypeState::Empty => return Ok(()),
        AccountTypeState::Snark(state) => state,
    };
    let vk = snark_state.verifier_key();
    let claim: Vec<u8> = compute_update_claim::<S>(acct_state, update.operation());
    let is_valid = vk
        .verify_claim_witness(&claim, update.update_proof())
        .is_ok();

    if !is_valid {
        return Err(StfError::InvalidUpdateProof { account_id: sender });
    }

    Ok(())
}

fn compute_update_claim<S: StateAccessor>(
    _acct_state: &<S as StateAccessor>::AccountState,
    _operation: &UpdateOperationData,
) -> Vec<u8> {
    // Use new state, processed messages, old state, refs and outputs to compute claim
    todo!()
}

pub(crate) fn verify_update_outputs_safe<S: StateAccessor>(
    outputs: &UpdateOutputs,
    sender: AccountId,
    acct_state: &S::AccountState,
    state_accessor: &S,
) -> StfResult<()> {
    let original_balance = acct_state.balance();
    let transfers = outputs.transfers();
    let messages = outputs.messages();

    // Check if sender exists
    if !state_accessor.check_account_exists(sender)? {
        return Err(StfError::NonExistentAccount(sender));
    }

    // Check if receivers exist
    for t in transfers {
        if !state_accessor.check_account_exists(t.dest())? {
            return Err(StfError::NonExistentAccount(t.dest()));
        }
    }

    for m in messages {
        if !state_accessor.check_account_exists(m.dest())? {
            return Err(StfError::NonExistentAccount(m.dest()));
        }
    }

    let total_sent = outputs
        .total_output_value()
        .ok_or(StfError::BitcoinAmountOverflow)?;

    // Check if there is sufficient balance.
    if total_sent > original_balance {
        return Err(StfError::InsufficientBalance);
    }
    Ok(())
}

pub(crate) fn verify_ledger_refs<S: StateAccessor>(
    _ledger_refs: &LedgerRefs,
    _state_accessor: &S,
) -> StfResult<()> {
    // TODO: implement this
    Ok(())
}

/// Type safe update that indicates it has been verified.
#[derive(Clone, Debug)]
pub(crate) struct VerifiedUpdate<'a> {
    operation: &'a UpdateOperationData,
}

impl<'a> VerifiedUpdate<'a> {
    pub(crate) fn operation(&self) -> &'a UpdateOperationData {
        self.operation
    }
}
