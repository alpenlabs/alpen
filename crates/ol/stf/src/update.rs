// TODO: this should probably exist in some of the account crates. Here for faster iteration.

use strata_acct_types::{
    AccountId, BitcoinAmount, MerkleProof, MsgPayload, StrataHasher, SystemAccount,
};
use strata_ledger_types::{
    AccountTypeState, Coin, IAccountState, IGlobalState, ISnarkAccountState, StateAccessor,
};
use strata_mmr::hasher::MerkleHasher;
use strata_ol_chain_types_new::OLLog;
use strata_snark_acct_types::{
    LedgerRefs, MessageEntry, SnarkAccountUpdate, UpdateOperationData, UpdateOutputs,
};

use crate::{
    error::{StfError, StfResult},
    msg_handlers::{
        handle_bridge_gateway_msg, handle_bridge_gateway_transfer, handle_snark_msg,
        handle_snark_transfer,
    },
};

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
        return Err(StfError::InvalidUpdateSequence);
    }

    // 2. Check message counts / proof indices line up
    let cur_idx = state.next_inbox_idx();
    let new_idx = operation.new_state().next_inbox_msg_idx();
    let processed_len = operation.processed_messages().len() as u64;

    if cur_idx
        .checked_add(processed_len)
        .ok_or(StfError::MsgIndexOverflow)?
        != new_idx
    {
        return Err(StfError::InvalidMsgIndex);
    }

    // 3. Verify ledger references using the provided state accessor
    verify_ledger_refs(operation.ledger_refs(), state_accessor)?;

    // Create message proofs
    let message_proofs = get_message_proofs::<S>(update, &state)?;
    // 4. Verify input mmr proofs
    verify_input_mmr_proofs::<S>(&state, &message_proofs)?;

    // 4. Verify outputs can be applied safely
    verify_update_outputs_safe(outputs, sender, acct_state, state_accessor)?;

    // 5. Verify the witness check
    verify_update_witness(acct_state, update, state_accessor)?;

    Ok(VerifiedUpdate { operation })
}

fn get_message_proofs<S: StateAccessor>(
    update: &SnarkAccountUpdate,
    state: &<S::AccountState as IAccountState>::SnarkAccountState,
) -> StfResult<Vec<(MessageEntry, MerkleProof)>> {
    let mut cur_idx = state.next_inbox_idx();
    let mut proofs = Vec::new();

    for msg in update.operation().processed_messages() {
        let proof = state
            .get_message_proof(msg, cur_idx)?
            .ok_or(StfError::NonExistentMessage(msg.clone()))?;

        let mproof = MerkleProof::from_cohashes(proof.raw_proof().cohashes().to_vec(), cur_idx);
        proofs.push((msg.clone(), mproof));
        cur_idx = cur_idx.checked_add(1).ok_or(StfError::MsgIndexOverflow)?;
    }
    Ok(proofs)
}

fn verify_input_mmr_proofs<S: StateAccessor>(
    state: &<S::AccountState as IAccountState>::SnarkAccountState,
    msg_proofs: &[(MessageEntry, MerkleProof)],
) -> StfResult<()> {
    for (msg, proof) in msg_proofs {
        let msg_bytes: Vec<u8> = msg.as_ssz_bytes();
        let hash = StrataHasher::hash_leaf(&msg_bytes);
        if !state.inbox_mmr().verify(proof, &hash) {
            return Err(StfError::NonExistentMessage(msg.clone()));
        }
    }
    Ok(())
}

fn verify_update_witness<S: StateAccessor>(
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
        return Err(StfError::InvalidUpdateProof);
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

fn verify_update_outputs_safe<S: StateAccessor>(
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

fn verify_ledger_refs<S: StateAccessor>(
    _ledger_refs: &LedgerRefs,
    _state_accessor: &S,
) -> StfResult<()> {
    // TODO: implement this
    Ok(())
}

pub(crate) fn apply_update_outputs<'a, S: StateAccessor>(
    state_accessor: &mut S,
    sender: AccountId,
    sender_state: &mut S::AccountState,
    verified_update: VerifiedUpdate<'a>,
) -> StfResult<Vec<OLLog>> {
    let outputs = verified_update.operation().outputs();
    let transfers = outputs.transfers();
    let messages = outputs.messages();

    // Process transfers
    for transfer in transfers {
        send_transfer(state_accessor, sender, transfer.dest(), transfer.value())?;
    }

    // Process messages
    for msg in messages {
        let payload = msg.payload();
        send_message(state_accessor, sender, msg.dest(), payload)?;
    }

    // Update balance
    let total_sent = outputs
        .total_output_value()
        .ok_or(StfError::BitcoinAmountOverflow)?;

    let _coins = sender_state.take_balance(total_sent);
    // TODO: do something with coins

    // Update account type state
    // TODO: think about where it makes sense to keep this logic.
    match sender_state.get_type_state_mut()? {
        AccountTypeState::Empty => {}
        AccountTypeState::Snark(st) => {
            let operation = verified_update.operation();
            let new_state = operation.new_state();
            st.set_proof_state_directly(
                new_state.inner_state(),
                new_state.next_inbox_msg_idx(),
                operation.seq_no(),
            );
        }
    };

    Ok(Vec::new()) // TODO: add logs, especially withdrawals
}

pub(crate) fn send_message<S: StateAccessor>(
    state_accessor: &mut S,
    from: AccountId,
    to: AccountId,
    msg_payload: &MsgPayload,
) -> StfResult<()> {
    let cur_epoch = state_accessor.global().cur_epoch();
    let Some(target_acct) = state_accessor.get_account_state_mut(to)? else {
        return Err(StfError::NonExistentAccount(to));
    };

    // First update the balance
    let coin = Coin::new_unchecked(msg_payload.value());
    target_acct.add_balance(coin);

    if let Some(sys_handler) = get_system_msg_handler::<S>(to) {
        return sys_handler(state_accessor, from, msg_payload);
    };

    match target_acct.get_type_state_mut()? {
        AccountTypeState::Empty => {
            // todo: what exactly to do?
            Ok(())
        }
        AccountTypeState::Snark(snark_state) => {
            handle_snark_msg::<S>(cur_epoch, snark_state, from, msg_payload)
        }
    }
}

pub(crate) fn send_transfer<S: StateAccessor>(
    state_accessor: &mut S,
    from: AccountId,
    to: AccountId,
    amt: BitcoinAmount,
) -> StfResult<()> {
    let cur_epoch = state_accessor.global().cur_epoch();
    let Some(target_acct) = state_accessor.get_account_state_mut(to)? else {
        return Err(StfError::NonExistentAccount(to));
    };

    // First update the balance
    let coin = Coin::new_unchecked(amt);
    target_acct.add_balance(coin);

    if let Some(sys_handler) = get_system_transfer_handler::<S>(to) {
        return sys_handler(state_accessor, from, amt);
    };

    match target_acct.get_type_state_mut()? {
        AccountTypeState::Empty => {
            // todo: what exactly to do?
            Ok(())
        }
        AccountTypeState::Snark(snark_state) => {
            handle_snark_transfer::<S>(cur_epoch, snark_state, from, amt)
        }
    }
}

type MsgHandler<S> = fn(&mut S, AccountId, &MsgPayload) -> StfResult<()>;

fn get_system_msg_handler<S: StateAccessor>(acct_id: AccountId) -> Option<MsgHandler<S>> {
    if acct_id == SystemAccount::Bridge.id() {
        Some(handle_bridge_gateway_msg)
    } else {
        None
    }
}

type TransferHandler<S> = fn(&mut S, AccountId, BitcoinAmount) -> StfResult<()>;

fn get_system_transfer_handler<S: StateAccessor>(acct_id: AccountId) -> Option<TransferHandler<S>> {
    if acct_id == SystemAccount::Bridge.id() {
        Some(handle_bridge_gateway_transfer)
    } else {
        None
    }
}

#[derive(Clone, Debug)]
pub(crate) struct VerifiedUpdate<'a> {
    operation: &'a UpdateOperationData,
}

impl<'a> VerifiedUpdate<'a> {
    pub(crate) fn operation(&self) -> &'a UpdateOperationData {
        self.operation
    }
}
