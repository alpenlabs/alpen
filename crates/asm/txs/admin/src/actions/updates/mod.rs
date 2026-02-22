pub mod multisig;
pub mod operator;
pub mod predicate;
pub mod seq;

use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_params::Role;

use crate::{
    actions::{
        Sighash,
        updates::{
            multisig::MultisigUpdate, operator::OperatorSetUpdate, predicate::PredicateUpdate,
            seq::SequencerUpdate,
        },
    },
    constants::AdminTxType,
};

/// An action that updates some part of the ASM.
#[derive(Clone, Debug, Eq, PartialEq, Arbitrary, BorshDeserialize, BorshSerialize)]
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

impl Sighash for UpdateAction {
    fn tx_type(&self) -> AdminTxType {
        match self {
            UpdateAction::Multisig(m) => m.tx_type(),
            UpdateAction::OperatorSet(o) => o.tx_type(),
            UpdateAction::Sequencer(s) => s.tx_type(),
            UpdateAction::VerifyingKey(v) => v.tx_type(),
        }
    }

    fn sighash_payload(&self) -> Vec<u8> {
        match self {
            UpdateAction::Multisig(m) => m.sighash_payload(),
            UpdateAction::OperatorSet(o) => o.sighash_payload(),
            UpdateAction::Sequencer(s) => s.sighash_payload(),
            UpdateAction::VerifyingKey(v) => v.sighash_payload(),
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
