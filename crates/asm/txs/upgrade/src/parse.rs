use strata_asm_common::TxInput;

use crate::{
    actions::{
        CancelAction, EnactAction, MultisigAction,
        updates::{
            multisig::MultisigUpdate, operator::OperatorSetUpdate, seq::SequencerUpdate,
            vk::VerifyingKeyUpdate,
        },
    },
    constants::{
        CANCEL_TX_TYPE, ENACT_TX_TYPE, MULTISIG_CONFIG_UPDATE_TX_TYPE, OPERATOR_UPDATE_TX_TYPE,
        SEQUENCER_UPDATE_TX_TYPE, VK_UPDATE_TX_TYPE,
    },
    crypto::AggregatedVote,
    error::UpgradeTxParseError,
};

pub fn parse_tx_multisig_action_and_vote(
    tx: &TxInput<'_>,
) -> Result<(MultisigAction, AggregatedVote), UpgradeTxParseError> {
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

        _ => Err(UpgradeTxParseError::UnknownTxType)?,
    };
    Ok((action, vote))
}
