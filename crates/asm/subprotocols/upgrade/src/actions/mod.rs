pub mod cancel;
pub mod id;
pub mod multisig_update;
pub mod operator_update;
pub mod seq_update;
pub mod vk_update;

use borsh::{BorshDeserialize, BorshSerialize};
use multisig_update::MultisigConfigUpdate;
use operator_update::OperatorSetUpdate;
use seq_update::SequencerUpdate;
use strata_primitives::hash::compute_borsh_hash;
use vk_update::VerifyingKeyUpdate;

use crate::{
    actions::id::ActionId,
    roles::{Role, StrataProof},
};

pub const MULTISIG_CONFIG_UPDATE_ENACTMENT_DELAY: u64 = 2_016;
pub const OPERATOR_UPDATE_ENACTMENT_DELAY: u64 = 2_016;
pub const SEQUENCER_UPDATE_ENACTMENT_DELAY: u64 = 2_016;
pub const ASM_VK_ENACTMENT_DELAY: u64 = 12_960;
pub const OL_STF_VK_ENACTMENT_DELAY: u64 = 4_320;

#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub enum UpgradeAction {
    Multisig(MultisigConfigUpdate),
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

impl From<MultisigConfigUpdate> for UpgradeAction {
    fn from(m: MultisigConfigUpdate) -> Self {
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

/// A pending upgrade action that will be triggered after a specified number
/// of Bitcoin blocks unless cancelled by a CancelTx.
///
/// The `blocks_remaining` counter is decremented by one for each new Bitcoin
/// block; when it reaches zero, the specified `upgrade` is automatically
/// enacted.
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct PendingUpgradeAction {
    id: ActionId,
    upgrade: UpgradeAction,
    blocks_remaining: u64,
    role: Role,
}

impl PendingUpgradeAction {
    pub fn new(upgrade: UpgradeAction, blocks_remaining: u64, role: Role) -> Self {
        let id = upgrade.compute_id();
        Self {
            id,
            upgrade,
            blocks_remaining,
            role,
        }
    }

    pub fn role(&self) -> &Role {
        &self.role
    }

    pub fn id(&self) -> &ActionId {
        &self.id
    }

    pub fn upgrade(&self) -> &UpgradeAction {
        &self.upgrade
    }

    pub fn blocks_remaining(&self) -> u64 {
        self.blocks_remaining
    }
    pub fn decrement_blocks_remaining(&mut self) {
        if self.blocks_remaining > 0 {
            self.blocks_remaining -= 1;
        }
    }
}
