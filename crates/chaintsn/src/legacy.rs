//! Legacy routines extracted from `StateCache`.

use bitcoin::block::Header;
use strata_asm_types::L1VerificationError;
use strata_btc_types::WithdrawalFulfillmentInfo;
use strata_ol_chainstate_types::Chainstate;
use strata_primitives::{
    bridge::{BitcoinBlockHeight, OperatorIdx},
    l1::*,
    prelude::*,
};
use strata_ol_chain_types::DepositIntent;
use strata_state::bridge_state::*;

use crate::{context::StateAccessor, macros::*};

#[derive(Debug)]
pub struct FauxStateCache<'s, S> {
    state: &'s mut S,
}

impl<'s, S: StateAccessor> FauxStateCache<'s, S> {
    pub fn new(state: &'s mut S) -> Self {
        Self { state }
    }

    pub fn inner(&self) -> &S {
        self.state
    }

    pub fn inner_mut(&mut self) -> &mut S {
        self.state
    }

    pub fn state(&self) -> &Chainstate {
        self.state.state_untracked()
    }

    fn state_mut(&mut self) -> &mut Chainstate {
        self.state.state_mut_untracked()
    }

    /// Update HeaderVerificationState
    pub fn update_header_vs(&mut self, header: &Header) -> Result<(), L1VerificationError> {
        self.state_mut()
            .l1_view_mut()
            .header_vs_mut()
            .check_and_update(header)
    }

    /// Writes a deposit intent into an execution environment's input queue.
    pub fn insert_deposit_intent(&mut self, ee_id: u32, intent: DepositIntent) {
        assert_eq!(ee_id, 0, "stateop: only support execution env 0 right now");
        self.state_mut()
            .exec_env_state_mut()
            .pending_deposits
            .push_back(intent);
    }

    /// Remove a deposit intent from the pending deposits queue.
    ///
    /// This actually removes possibly multiple deposit intents.
    pub fn consume_deposit_intent(&mut self, idx: u64) {
        let deposits = self.state_mut().exec_env_state_mut().pending_deposits_mut();

        let front_idx = deposits
            .front_idx()
            .expect("stateop: empty deposit intent queue");

        // deposit intent indices processed sequentially, without any gaps
        let to_drop_count = idx
            .checked_sub(front_idx) // ensures to_drop_idx >= front_idx
            .expect("stateop: unable to consume deposit intent")
            + 1;

        deposits
            .pop_front_n_vec(to_drop_count as usize) // ensures to_drop_idx < front_idx + len
            .expect("stateop: unable to consume deposit intent");
    }

    /// Inserts a new operator with the specified pubkeys into the operator table.
    pub fn insert_operator(&mut self, signing_pk: Buf32, wallet_pk: Buf32) {
        self.state_mut()
            .operator_table_mut()
            .insert(signing_pk, wallet_pk);
    }

    /// Inserts a new deposit with some settings.
    pub fn insert_deposit_entry(
        &mut self,
        idx: u32,
        tx_ref: OutputRef,
        amt: BitcoinAmount,
        operators: Vec<OperatorIdx>,
    ) -> bool {
        let dt = self.state_mut().deposits_table_mut();
        dt.try_create_deposit_at(idx, tx_ref, operators, amt)
    }

    /// Assigns a withdrawal command to a deposit, with an expiration.
    pub fn assign_withdrawal_command(
        &mut self,
        deposit_idx: u32,
        operator_idx: OperatorIdx,
        cmd: DispatchCommand,
        exec_height: BitcoinBlockHeight,
        withdrawal_txid: Buf32,
    ) {
        let deposit_ent = self
            .state_mut()
            .deposits_table_mut()
            .get_deposit_mut(deposit_idx)
            .expect("stateop: missing deposit idx");

        let state =
            DepositState::Dispatched(DispatchedState::new(cmd.clone(), operator_idx, exec_height));
        deposit_ent.set_state(state);
        deposit_ent.set_withdrawal_request_txid(Some(withdrawal_txid));
    }

    /// Updates the deposit assignee and expiration date.
    pub fn reset_deposit_assignee(
        &mut self,
        deposit_idx: u32,
        operator_idx: OperatorIdx,
        new_exec_height: BitcoinBlockHeight,
    ) {
        let deposit_ent = self
            .state_mut()
            .deposits_table_mut()
            .get_deposit_mut(deposit_idx)
            .expect("stateop: missing deposit idx");

        if let DepositState::Dispatched(dstate) = deposit_ent.deposit_state_mut() {
            dstate.set_assignee(operator_idx);
            dstate.set_exec_deadline(new_exec_height);
        } else {
            panic!("stateop: unexpected deposit state");
        };
    }

    /// Updates the deposit state to Fulfilled.
    pub fn mark_deposit_fulfilled(&mut self, winfo: &WithdrawalFulfillmentInfo) {
        let deposit_ent = self.deposit_entry_mut_expect(winfo.deposit_idx);

        let oidx = winfo.operator_idx;
        let is_valid = deposit_ent.deposit_state().is_dispatched_to(oidx);

        assert!(is_valid, "stateop: incorrect deposit state dispatch");

        deposit_ent.set_state(DepositState::Fulfilled(FulfilledState::new(
            winfo.operator_idx,
            winfo.amt,
            winfo.txid,
        )));
    }

    // Updates the deposit state as Reimbursed.
    pub fn mark_deposit_reimbursed(&mut self, deposit_idx: u32) {
        let deposit_ent = self.deposit_entry_mut_expect(deposit_idx);

        if !matches!(deposit_ent.deposit_state(), DepositState::Fulfilled(_)) {
            // TODO: handle this better after TN1 bridge is integrated
            warn!("stateop: deposit spent at unexpected state");
        }

        deposit_ent.set_state(DepositState::Reimbursed);
    }

    fn deposit_entry_mut_expect(&mut self, deposit_idx: u32) -> &mut DepositEntry {
        self.state_mut()
            .deposits_table_mut()
            .get_deposit_mut(deposit_idx)
            .expect("stateop: missing deposit idx")
    }
}
