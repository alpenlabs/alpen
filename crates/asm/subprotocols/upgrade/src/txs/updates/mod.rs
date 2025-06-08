use borsh::{BorshDeserialize, BorshSerialize};
use strata_primitives::hash::compute_borsh_hash;

use crate::{
    actions::id::ActionId,
    roles::{Role, StrataProof},
    txs::updates::{
        multisig::MultisigUpdate, operator::OperatorSetUpdate, seq::SequencerUpdate,
        vk::VerifyingKeyUpdate,
    },
};

pub mod multisig;
pub mod operator;
pub mod seq;
pub mod vk;

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
