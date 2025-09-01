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

/// An action that upgrades some part of the ASM
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary)]
pub enum UpgradeAction {
    Multisig(MultisigUpdate),
    OperatorSet(OperatorSetUpdate),
    Sequencer(SequencerUpdate),
    VerifyingKey(VerifyingKeyUpdate),
}

impl UpgradeAction {
    /// The role authorized to enact this upgrade.
    pub fn required_role(&self) -> Role {
        match self {
            UpgradeAction::Multisig(m) => m.role(),
            UpgradeAction::OperatorSet(_) => Role::BridgeAdmin,
            UpgradeAction::Sequencer(_) => Role::StrataAdmin,
            UpgradeAction::VerifyingKey(v) => match v.kind() {
                ProofType::Asm => Role::BridgeConsensusManager,
                ProofType::OlStf => Role::StrataConsensusManager,
            },
        }
    }
}

// Allow easy conversion from each update type into a unified `UpgradeAction`.
impl From<MultisigUpdate> for UpgradeAction {
    fn from(update: MultisigUpdate) -> Self {
        UpgradeAction::Multisig(update)
    }
}

impl From<OperatorSetUpdate> for UpgradeAction {
    fn from(update: OperatorSetUpdate) -> Self {
        UpgradeAction::OperatorSet(update)
    }
}

impl From<SequencerUpdate> for UpgradeAction {
    fn from(update: SequencerUpdate) -> Self {
        UpgradeAction::Sequencer(update)
    }
}

impl From<VerifyingKeyUpdate> for UpgradeAction {
    fn from(update: VerifyingKeyUpdate) -> Self {
        UpgradeAction::VerifyingKey(update)
    }
}
