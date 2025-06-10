use borsh::{BorshDeserialize, BorshSerialize};

pub const CANCEL_TX_TYPE: u8 = 0;
pub const ENACT_TX_TYPE: u8 = 1;

pub const MULTISIG_CONFIG_UPDATE_TX_TYPE: u8 = 10;
pub const OPERATOR_UPDATE_TX_TYPE: u8 = 11;
pub const SEQUENCER_UPDATE_TX_TYPE: u8 = 12;
pub const VK_UPDATE_TX_TYPE: u8 = 13;

mod cancel;
mod enact;
mod updates;

pub use cancel::CancelAction;
pub use enact::EnactAction;
use strata_asm_common::TxInput;
pub use updates::{UpgradeAction, id::UpdateId};

use crate::{
    error::{DeserializeError, UpgradeError},
    multisig::vote::AggregatedVote,
    txs::updates::{
        multisig::MultisigUpdate, operator::OperatorSetUpdate, seq::SequencerUpdate,
        vk::VerifyingKeyUpdate,
    },
};

/// A high‐level multisig operation that participants can propose.
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub enum MultisigAction {
    /// Cancel a pending action
    Cancel(CancelAction),
    /// Execute a committed action
    Enact(EnactAction),
    /// Propose an upgrade
    Upgrade(UpgradeAction),
}

pub fn parse_tx_multisig_action_and_vote(
    tx: &TxInput<'_>,
) -> Result<(MultisigAction, AggregatedVote), UpgradeError> {
    let vote = AggregatedVote::extract_from_tx(tx)?;

    let action = match tx.tag().tx_type() {
        CANCEL_TX_TYPE => MultisigAction::Cancel(CancelAction::extract_from_tx(tx)?),
        ENACT_TX_TYPE => MultisigAction::Enact(EnactAction::extract_from_tx(tx)?),

        MULTISIG_CONFIG_UPDATE_TX_TYPE => {
            MultisigAction::Upgrade(MultisigUpdate::extract_from_tx(tx)?.into())
        }
        OPERATOR_UPDATE_TX_TYPE => {
            MultisigAction::Upgrade(OperatorSetUpdate::extract_from_tx(tx)?.into())
        }
        SEQUENCER_UPDATE_TX_TYPE => {
            MultisigAction::Upgrade(SequencerUpdate::extract_from_tx(tx)?.into())
        }
        VK_UPDATE_TX_TYPE => {
            MultisigAction::Upgrade(VerifyingKeyUpdate::extract_from_tx(tx)?.into())
        }

        _ => Err(DeserializeError::MalformedTransaction(1))?, // FIXME:
    };
    Ok((action, vote))
}
