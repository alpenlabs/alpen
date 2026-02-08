pub mod multisig;
pub mod operator;
pub mod predicate;
pub mod seq;

use arbitrary::Arbitrary;
use strata_primitives::roles::Role;

use crate::actions::updates::{
    multisig::MultisigUpdate, operator::OperatorSetUpdate, predicate::PredicateUpdate,
    seq::SequencerUpdate,
};

/// An action that updates some part of the ASM.
#[derive(
    Clone, Debug, Eq, PartialEq, Arbitrary, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub enum UpdateAction {
    Multisig(MultisigUpdate),
    OperatorSet(OperatorSetUpdate),
    Sequencer(SequencerUpdate),
    VerifyingKey(PredicateUpdate),
}

impl UpdateAction {
    /// The role authorized to enact this update.
    pub fn required_role(&self) -> Role {
        match self {
            UpdateAction::Multisig(m) => m.role(),
            UpdateAction::OperatorSet(_) => Role::StrataAdministrator,
            UpdateAction::VerifyingKey(_) => Role::StrataAdministrator,
            UpdateAction::Sequencer(_) => Role::StrataSequencerManager,
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

impl From<PredicateUpdate> for UpdateAction {
    fn from(update: PredicateUpdate) -> Self {
        UpdateAction::VerifyingKey(update)
    }
}
