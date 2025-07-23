//! Deposit state types and state transitions.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_primitives::{
    bridge::{BitcoinBlockHeight, OperatorIdx},
    buf::Buf32,
    l1::BitcoinAmount,
};

use super::withdrawal::DispatchCommand;

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DepositState {
    /// Deposit utxo has been recognized.
    Created(CreatedState),

    /// Deposit utxo has been accepted.
    Accepted,

    /// Order to send out withdrawal dispatched.
    Dispatched(DispatchedState),

    /// Withdrawal is being processed by the assigned operator.
    Fulfilled(FulfilledState),

    /// Executed state, will be cleaned up.
    Reimbursed,
}

impl DepositState {
    pub fn is_dispatched_to(&self, operator_idx: u32) -> bool {
        matches!(self, DepositState::Dispatched(s) if s.assignee() == operator_idx)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
pub struct CreatedState {
    /// Destination identifier in EL, probably an encoded address.
    dest_ident: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
pub struct DispatchedState {
    /// Configuration for outputs to be written to.
    cmd: DispatchCommand,

    /// The index of the operator that's fronting the funds for the withdrawal,
    /// and who will be reimbursed by the bridge notaries.
    assignee: OperatorIdx,

    /// L1 block height before which we expect the dispatch command to be
    /// executed and after which this assignment command is no longer valid.
    ///
    /// If a checkpoint is processed for this L1 height and the withdrawal still
    /// goes out it won't be honored.
    exec_deadline: BitcoinBlockHeight,
}

impl DispatchedState {
    pub fn new(
        cmd: DispatchCommand,
        assignee: OperatorIdx,
        exec_deadline: BitcoinBlockHeight,
    ) -> Self {
        Self {
            cmd,
            assignee,
            exec_deadline,
        }
    }

    pub fn cmd(&self) -> &DispatchCommand {
        &self.cmd
    }

    pub fn assignee(&self) -> OperatorIdx {
        self.assignee
    }

    pub fn exec_deadline(&self) -> BitcoinBlockHeight {
        self.exec_deadline
    }

    pub fn set_assignee(&mut self, assignee_op_idx: OperatorIdx) {
        self.assignee = assignee_op_idx;
    }

    pub fn set_exec_deadline(&mut self, exec_deadline: BitcoinBlockHeight) {
        self.exec_deadline = exec_deadline;
    }
}

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
pub struct FulfilledState {
    /// The index of the operator that has fronted the funds for the withdrawal,
    /// and who will be reimbursed by the bridge notaries.
    assignee: OperatorIdx,

    /// Actual amount sent in withdrawal
    amt: BitcoinAmount,

    /// Corresponding bitcoin transaction id
    txid: Buf32,
}

impl FulfilledState {
    pub fn new(assignee: OperatorIdx, amt: BitcoinAmount, txid: Buf32) -> Self {
        Self {
            assignee,
            amt,
            txid,
        }
    }

    pub fn assignee(&self) -> OperatorIdx {
        self.assignee
    }

    pub fn amt(&self) -> BitcoinAmount {
        self.amt
    }
}
