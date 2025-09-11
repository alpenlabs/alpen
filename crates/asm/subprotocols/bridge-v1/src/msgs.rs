use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::{InterprotoMsg, SubprotocolId};

use crate::{BRIDGE_V1_SUBPROTOCOL_ID, state::withdrawal::WithdrawOutput};

/// Incoming message types received from other subprotocols.
///
/// Since we assume that all subprotocols have a cooperative relationship, we simply
/// act on the incoming message without verifying its source. The duty to craft valid
/// messages falls to the sending subprotocol.
///
/// This enum represents all possible message types that the bridge subprotocol can
/// receive from other subprotocols in the ASM.
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub enum BridgeIncomingMsg {
    /// This message is emitted by the Checkpoint subprotocol after a checkpoint proof
    /// has been validated. It contains the withdrawal command specifying:
    /// - The Bitcoin address where the funds should be sent (as a descriptor)
    /// - The amount requested to be to be withdrawn
    ///
    /// Note: The actual amount received by the user will be less than specified due to
    /// operator fees being deducted from the withdrawal amount.
    ///
    /// Upon receiving this message, the Bridge subprotocol will create a withdrawal assignment
    /// by selecting an unassigned deposit and assigning it to a random operator.
    DispatchWithdrawal(WithdrawOutput),
}

impl InterprotoMsg for BridgeIncomingMsg {
    fn id(&self) -> SubprotocolId {
        BRIDGE_V1_SUBPROTOCOL_ID
    }

    fn as_dyn_any(&self) -> &dyn std::any::Any {
        self
    }
}
