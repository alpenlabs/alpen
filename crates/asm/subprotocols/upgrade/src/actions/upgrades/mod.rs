use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::TxInput;
use strata_primitives::hash::compute_borsh_hash;

use crate::{
    actions::{
        id::ActionId,
        upgrades::{
            multisig::MultisigUpdate, operator::OperatorSetUpdate, seq::SequencerUpdate,
            vk::VerifyingKeyUpdate,
        },
    },
    error::UpgradeError,
    multisig::{msg::MultisigOp, vote::AggregatedVote},
    roles::{Role, StrataProof},
    state::UpgradeSubprotoState,
};

pub mod multisig;
pub mod operator;
pub mod seq;
pub mod vk;

pub const MULTISIG_CONFIG_UPDATE_ENACTMENT_DELAY: u64 = 2_016;
pub const OPERATOR_UPDATE_ENACTMENT_DELAY: u64 = 2_016;
pub const SEQUENCER_UPDATE_ENACTMENT_DELAY: u64 = 2_016;
pub const ASM_VK_ENACTMENT_DELAY: u64 = 12_960;
pub const OL_STF_VK_ENACTMENT_DELAY: u64 = 4_320;

pub const MULTISIG_CONFIG_UPDATE_TX_TYPE: u8 = 1;
pub const OPERATOR_UPDATE_TX_TYPE: u8 = 2;
pub const SEQUENCER_UPDATE_TX_TYPE: u8 = 3;
pub const VK_UPDATE_TX_TYPE: u8 = 4;

#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub enum UpgradeAction {
    Multisig(MultisigUpdate),
    OperatorSet(OperatorSetUpdate),
    Sequencer(SequencerUpdate),
    VerifyingKey(VerifyingKeyUpdate),
}

impl UpgradeAction {
    pub fn compute_id(&self) -> ActionId {
        compute_borsh_hash(&self).into()
    }

    /// Enactment delay for the action
    pub fn enactment_delay(&self) -> u64 {
        match self {
            UpgradeAction::Multisig(_) => MULTISIG_CONFIG_UPDATE_ENACTMENT_DELAY,
            UpgradeAction::OperatorSet(_) => OPERATOR_UPDATE_ENACTMENT_DELAY,
            UpgradeAction::Sequencer(_) => SEQUENCER_UPDATE_ENACTMENT_DELAY,
            UpgradeAction::VerifyingKey(v) => match v.proof_kind() {
                StrataProof::ASM => ASM_VK_ENACTMENT_DELAY,
                StrataProof::OlStf => OL_STF_VK_ENACTMENT_DELAY,
            },
        }
    }

    /// Role which has the authority to enact this action.
    pub fn role(&self) -> Role {
        match self {
            UpgradeAction::Multisig(m) => m.role(),
            UpgradeAction::OperatorSet(_) => Role::BridgeAdmin,
            UpgradeAction::Sequencer(_) => Role::StrataAdmin,
            UpgradeAction::VerifyingKey(v) => match v.proof_kind() {
                StrataProof::ASM => Role::BridgeConsensusManager,
                StrataProof::OlStf => Role::StrataConsensusManager,
            },
        }
    }
}

impl From<MultisigUpdate> for UpgradeAction {
    fn from(m: MultisigUpdate) -> Self {
        UpgradeAction::Multisig(m)
    }
}

impl From<OperatorSetUpdate> for UpgradeAction {
    fn from(o: OperatorSetUpdate) -> Self {
        UpgradeAction::OperatorSet(o)
    }
}

impl From<SequencerUpdate> for UpgradeAction {
    fn from(s: SequencerUpdate) -> Self {
        UpgradeAction::Sequencer(s)
    }
}

impl From<VerifyingKeyUpdate> for UpgradeAction {
    fn from(v: VerifyingKeyUpdate) -> Self {
        UpgradeAction::VerifyingKey(v)
    }
}

/// Handles an incoming upgrade transaction:
pub fn handle_update_tx(
    state: &mut UpgradeSubprotoState,
    tx: &TxInput<'_>,
) -> Result<(), UpgradeError> {
    // Extract the aggregated vote from the transaction payload
    let vote = AggregatedVote::extract_from_tx(tx)?;

    // Parse the transaction into a concrete UpgradeAction based on its type tag
    let action: UpgradeAction = match tx.tag().tx_type() {
        VK_UPDATE_TX_TYPE => {
            // Extract a VerifyingKeyUpdate and wrap it
            Ok(UpgradeAction::from(VerifyingKeyUpdate::extract_from_tx(
                tx,
            )?))
        }
        SEQUENCER_UPDATE_TX_TYPE => {
            // Extract a SequencerUpdate and wrap it
            Ok(UpgradeAction::from(SequencerUpdate::extract_from_tx(tx)?))
        }
        MULTISIG_CONFIG_UPDATE_TX_TYPE => {
            // Extract a MultisigUpdate and wrap it
            Ok(UpgradeAction::from(MultisigUpdate::extract_from_tx(tx)?))
        }
        OPERATOR_UPDATE_TX_TYPE => {
            // Extract an OperatorSetUpdate and wrap it
            Ok(UpgradeAction::from(OperatorSetUpdate::extract_from_tx(tx)?))
        }
        // Unknown transaction type: cannot determine the upgrade action
        _ => Err(UpgradeError::UnknownRole),
    }?;

    // Retrieve the authority entity responsible for this action's role
    let authority = state
        .get_authority(&action.role())
        .ok_or(UpgradeError::UnknownRole)?;

    // If this is a multisig configuration update, ensure the new config is valid
    if let UpgradeAction::Multisig(update) = &action {
        authority.config().validate_update(update.config_update())?;
    }

    // Convert the action into a multisig operation and validate it against the vote
    let op = MultisigOp::from(action.clone());
    authority.validate_op(&op, &vote)?;

    // Create a pending upgrade action and enqueue it for later enactment
    let pending_action = super::PendingUpgradeAction::from(action);
    state.add_pending_action(pending_action);

    Ok(())
}
