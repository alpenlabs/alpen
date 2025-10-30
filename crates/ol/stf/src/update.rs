// TODO: this should probably exist in some of the account crates. Here for faster iteration.

use std::any::Any;

use strata_acct_types::{
    AccountId, AccountTypeId, BitcoinAmount, MerkleProof, MsgPayload, StrataHasher, SystemAccount,
};
use strata_ledger_types::{
    AccountTypeState, Coin, IAccountState, ISnarkAccountState, StateAccessor,
};
use strata_mmr::{Sha256Hasher, hasher::MerkleHasher};
use strata_ol_chain_types_new::OLLog;
use strata_ol_state_types::OLState;
use strata_snark_acct_types::{
    LedgerRefs, MessageEntry, MessageEntryProof, SnarkAccountUpdate, UpdateOutputs,
};

use crate::{
    error::{StfError, StfResult},
    msg_handlers::{handle_bridge_gateway_msg, handle_snark_msg, handle_snark_transfer},
};

/// Verifies an account update is correct with respect to the current state of
/// snark account, including checking account balances.
pub(crate) fn verify_update_correctness<'a, S: StateAccessor<GlobalState = OLState>>(
    state_accessor: &S,
    sender: AccountId,
    acct_state: &S::AccountState,
    update: &'a SnarkAccountUpdate,
) -> StfResult<VerifiedOutputs<'a>> {
    let type_state = acct_state.get_type_state().unwrap();
    let operation = update.operation();
    let outputs = operation.outputs();
    let state = match type_state {
        AccountTypeState::Empty => {
            return Ok(VerifiedOutputs { inner: outputs });
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
        .checked_add(processed_len).expect("Msg index overflow") // TODO: expect
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

    Ok(VerifiedOutputs { inner: outputs })
}

fn get_message_proofs<S: StateAccessor<GlobalState = OLState>>(
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
        cur_idx += 1;
    }
    Ok(proofs)
}

fn verify_input_mmr_proofs<S: StateAccessor<GlobalState = OLState>>(
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

fn verify_update_witness<S: StateAccessor<GlobalState = OLState>>(
    _acct_state: &<S as StateAccessor>::AccountState,
    _update: &SnarkAccountUpdate,
    _state_accessor: &S,
) -> StfResult<()> {
    todo!()
}

fn verify_update_outputs_safe<S: StateAccessor<GlobalState = OLState>>(
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
        .expect("BitcoinAmount overflow");

    // Check if there is sufficient balance.
    if total_sent > original_balance {
        return Err(StfError::InsufficientBalance);
    }
    Ok(())
}

fn verify_ledger_refs<S: StateAccessor<GlobalState = OLState>>(
    _ledger_refs: &LedgerRefs,
    _state_accessor: &S,
) -> StfResult<()> {
    // TODO: implement this
    Ok(())
}

pub(crate) fn apply_update_outputs<'a, S: StateAccessor<GlobalState = OLState>>(
    state_accessor: &mut S,
    sender: AccountId,
    sender_state: &mut S::AccountState,
    verified_outs: VerifiedOutputs<'a>,
) -> StfResult<Vec<OLLog>> {
    let outputs = verified_outs.inner();
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
        .expect("BitcoinAmount overflow");

    let _coins = sender_state.take_balance(total_sent);
    // TODO: do something with coins

    Ok(Vec::new()) // TODO: add logs
}

fn send_message<S: StateAccessor<GlobalState = OLState>>(
    state_accessor: &mut S,
    from: AccountId,
    to: AccountId,
    msg_payload: &MsgPayload,
) -> StfResult<()> {
    let cur_epoch = state_accessor.global().cur_epoch() as u32;
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

fn send_transfer<S: StateAccessor<GlobalState = OLState>>(
    state_accessor: &mut S,
    from: AccountId,
    to: AccountId,
    amt: BitcoinAmount,
) -> StfResult<()> {
    let cur_epoch = state_accessor.global().cur_epoch() as u32;
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

fn get_system_msg_handler<S: StateAccessor<GlobalState = OLState>>(
    acct_id: AccountId,
) -> Option<MsgHandler<S>> {
    if acct_id == SystemAccount::Bridge.id() {
        Some(handle_bridge_gateway_msg)
    } else {
        None
    }
}

type TransferHandler<S> = fn(&mut S, AccountId, BitcoinAmount) -> StfResult<()>;

fn get_system_transfer_handler<S: StateAccessor<GlobalState = OLState>>(
    acct_id: AccountId,
) -> Option<TransferHandler<S>> {
    if acct_id == SystemAccount::Bridge.id() {
        panic!("We don't handle system transfers yet");
    } else {
        None
    }
}

/// Verified outputs. Just a wrapper that can be constructed only by the `verify_update_output`
/// function.
#[derive(Clone, Debug)]
pub(crate) struct VerifiedOutputs<'a> {
    inner: &'a UpdateOutputs,
    // TODO: Add total_sent as a computed value so that we don't have to compute it later while
    // applying
}

impl<'a> VerifiedOutputs<'a> {
    pub(crate) fn inner(self) -> &'a UpdateOutputs {
        self.inner
    }
}
