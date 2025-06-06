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
pub const VK_UPDATE_TX_TYPE: u8 = 2;
pub const OPERATOR_UPDATE_TX_TYPE: u8 = 3;
pub const SEQUENCER_UPDATE_TX_TYPE: u8 = 4;

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
            UpgradeAction::Multisig(_) => Role::BridgeAdmin,
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

pub fn handle_update_tx(
    state: &mut UpgradeSubprotoState,
    tx: &TxInput<'_>,
) -> Result<(), UpgradeError> {
    let vote = AggregatedVote::extract_from_tx(tx)?;

    // Build a Result<UpgradeAction, UpgradeError> via match, then unwrap with `?`.
    let action: UpgradeAction = match tx.tag().tx_type() {
        VK_UPDATE_TX_TYPE => {
            // extract_from_tx(tx)? yields VerifyingKeyUpdate
            // UpgradeAction::from(...) wraps it correctly
            Ok(UpgradeAction::from(VerifyingKeyUpdate::extract_from_tx(
                tx,
            )?))
        }
        SEQUENCER_UPDATE_TX_TYPE => Ok(UpgradeAction::from(SequencerUpdate::extract_from_tx(tx)?)),
        MULTISIG_CONFIG_UPDATE_TX_TYPE => {
            // Note: no extra `.into()` here
            Ok(UpgradeAction::from(MultisigUpdate::extract_from_tx(tx)?))
        }
        OPERATOR_UPDATE_TX_TYPE => {
            // Likewise, drop the redundant `.into()`
            Ok(UpgradeAction::from(OperatorSetUpdate::extract_from_tx(tx)?))
        }
        _ => Err(UpgradeError::UnknownRole),
    }?;

    let authority = state
        .get_authority(&action.role())
        .ok_or(UpgradeError::UnknownRole)?; // TODO: better error name

    if let UpgradeAction::Multisig(update) = &action {
        authority.config().validate_update(update.config_update())?;
    }

    let op = MultisigOp::from(action.clone());
    authority.validate_op(&op, &vote)?;

    // Create the pending action and enqueue it
    let pending_action = super::PendingUpgradeAction::from(action);
    state.add_pending_action(pending_action);

    Ok(())
}
