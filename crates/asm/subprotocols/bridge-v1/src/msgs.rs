use borsh::{BorshDeserialize, BorshSerialize};

use crate::state::withdrawal::WithdrawalCommand;

/// Message type that we receive messages from other subprotocols using.
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub enum BridgeIncomingMsg {
    ProcessWithdrawal(WithdrawalCommand),
}
