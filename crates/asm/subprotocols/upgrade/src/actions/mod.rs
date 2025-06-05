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
use vk_update::VerifyingKeyUpdate;

use crate::roles::Role;

#[derive(Debug, Clone, Eq, PartialEq, PartialOrd, Ord, Hash, BorshSerialize, BorshDeserialize)]
pub struct ActionId(pub [u8; 32]);

#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub enum UpgradeAction {
    Cancel(CancelAction),
    Multisig(MultisigConfigUpdate),
    OperatorSet(OperatorSetUpdate),
    Sequencer(SequencerUpdate),
    VerifyingKey(VerifyingKeyUpdate),
}

/// A pending upgrade action that will be triggered after a specified number
/// of Bitcoin blocks unless cancelled by a CancelTx.
///
/// The `blocks_remaining` counter is decremented by one for each new Bitcoin
/// block; when it reaches zero, the specified `upgrade` is automatically
/// enacted.
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct PendingUpgradeAction {
    upgrade: UpgradeAction,
    blocks_remaining: u64,
    role: Role,
}
