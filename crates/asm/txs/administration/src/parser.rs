use strata_asm_common::TxInputRef;
use strata_crypto::multisig::{Signature, vote::AggregatedVote};

use crate::{
    actions::{
        CancelAction, MultisigAction,
        updates::{
            multisig::MultisigUpdate, operator::OperatorSetUpdate, seq::SequencerUpdate,
            vk::VerifyingKeyUpdate,
        },
    },
    constants::{
        CANCEL_TX_TYPE, MULTISIG_CONFIG_UPDATE_TX_TYPE, OPERATOR_UPDATE_TX_TYPE,
        SEQUENCER_UPDATE_TX_TYPE, VK_UPDATE_TX_TYPE,
    },
    error::AdministrationTxParseError,
};

pub fn parse_tx_multisig_action_and_vote(
    tx: &TxInputRef<'_>,
) -> Result<(MultisigAction, AggregatedVote), AdministrationTxParseError> {
    let vote = parse_aggregated_vote(tx)?;

    let action = match tx.tag().tx_type() {
        CANCEL_TX_TYPE => MultisigAction::Cancel(CancelAction::extract_from_tx(tx)?),

        MULTISIG_CONFIG_UPDATE_TX_TYPE => {
            MultisigAction::Update(MultisigUpdate::extract_from_tx(tx)?.into())
        }
        OPERATOR_UPDATE_TX_TYPE => {
            MultisigAction::Update(OperatorSetUpdate::extract_from_tx(tx)?.into())
        }
        SEQUENCER_UPDATE_TX_TYPE => {
            MultisigAction::Update(SequencerUpdate::extract_from_tx(tx)?.into())
        }
        VK_UPDATE_TX_TYPE => {
            MultisigAction::Update(VerifyingKeyUpdate::extract_from_tx(tx)?.into())
        }

        _ => Err(AdministrationTxParseError::UnknownTxType)?,
    };
    Ok((action, vote))
}

/// Extracts the AggregatedVote from a transaction input.
/// FIXME: This is a placeholder function and should be replaced with actual logic.
pub fn parse_aggregated_vote(
    _tx: &TxInputRef<'_>,
) -> Result<AggregatedVote, AdministrationTxParseError> {
    Ok(AggregatedVote::new(vec![], Signature::default()))
}
