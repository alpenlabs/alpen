pub mod cancel;
pub mod multisig_update;
pub mod operator_update;
pub mod seq_update;
pub mod vk_update;

use borsh::{BorshDeserialize, BorshSerialize};
use cancel::CancelAction;
use multisig_update::MultisigConfigUpdate;
use operator_update::OperatorSetUpdate;
use seq_update::SequencerUpdate;
use strata_primitives::{buf::Buf32, hash::compute_borsh_hash};
use vk_update::VerifyingKeyUpdate;

use crate::roles::Role;

#[derive(
    Debug, Clone, Copy, Eq, PartialEq, PartialOrd, Ord, Hash, BorshSerialize, BorshDeserialize,
)]
pub struct ActionId(Buf32);

// Convert from Buf32 into ActionId
impl From<Buf32> for ActionId {
    fn from(bytes: Buf32) -> Self {
        ActionId(bytes)
    }
}

// Convert from ActionId back into Buf32
impl From<ActionId> for Buf32 {
    fn from(action_id: ActionId) -> Self {
        action_id.0
    }
}

impl ActionId {
    pub fn as_buf32(&self) -> &Buf32 {
        &self.0
    }
}

#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub enum UpgradeAction {
    Cancel(CancelAction),
    Multisig(MultisigConfigUpdate),
    OperatorSet(OperatorSetUpdate),
    Sequencer(SequencerUpdate),
    VerifyingKey(VerifyingKeyUpdate),
}

impl From<CancelAction> for UpgradeAction {
    fn from(action: CancelAction) -> Self {
        UpgradeAction::Cancel(action)
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

impl UpgradeAction {
    pub fn compute_id(&self) -> ActionId {
        compute_borsh_hash(&self).into()
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
