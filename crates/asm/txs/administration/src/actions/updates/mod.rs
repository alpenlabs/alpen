pub mod multisig;
pub mod operator;
pub mod seq;
pub mod vk;

use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use strata_primitives::roles::{ProofType, Role};

use crate::actions::updates::{
    multisig::MultisigUpdate, operator::OperatorSetUpdate, seq::SequencerUpdate,
    vk::VerifyingKeyUpdate,
};

/// An action that updates some part of the ASM
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary)]
pub enum UpdateAction {
    Multisig(MultisigUpdate),
    OperatorSet(OperatorSetUpdate),
    Sequencer(SequencerUpdate),
    VerifyingKey(VerifyingKeyUpdate),
}

impl UpdateAction {
    /// The role authorized to enact this update.
    pub fn required_role(&self) -> Role {
        match self {
            UpdateAction::Multisig(m) => m.role(),
            UpdateAction::OperatorSet(_) => Role::BridgeAdmin,
            UpdateAction::Sequencer(_) => Role::StrataAdmin,
            UpdateAction::VerifyingKey(v) => match v.kind() {
                ProofType::Asm => Role::BridgeConsensusManager,
                ProofType::OlStf => Role::StrataConsensusManager,
            },
        }
    }
}

// Allow easy conversion from each update type into a unified `UpdateAction`.
impl From<MultisigUpdate> for UpdateAction {
    fn from(update: MultisigUpdate) -> Self {
        UpdateAction::Multisig(update)
    }
}

impl From<OperatorSetUpdate> for UpdateAction {
    fn from(update: OperatorSetUpdate) -> Self {
        UpdateAction::OperatorSet(update)
    }
}

impl From<SequencerUpdate> for UpdateAction {
    fn from(update: SequencerUpdate) -> Self {
        UpdateAction::Sequencer(update)
    }
}

impl From<VerifyingKeyUpdate> for UpdateAction {
    fn from(update: VerifyingKeyUpdate) -> Self {
        UpdateAction::VerifyingKey(update)
    }
}
