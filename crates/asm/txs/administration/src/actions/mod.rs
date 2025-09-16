use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use strata_l1_txfmt::TxType;
use strata_primitives::{
    hash,
    roles::{ProofType, Role},
};

mod cancel;
pub mod updates;

pub use cancel::CancelAction;
use strata_primitives::{buf::Buf32, hash::compute_borsh_hash};
pub use updates::UpdateAction;

use crate::constants::{
    ASM_STF_VK_UPDATE_TX_TYPE, CANCEL_TX_TYPE, OL_STF_VK_UPDATE_TX_TYPE, OPERATOR_UPDATE_TX_TYPE,
    SEQUENCER_UPDATE_TX_TYPE, STRATA_ADMIN_MULTISIG_UPDATE_TX_TYPE,
    STRATA_SEQ_MANAGER_MULTISIG_UPDATE_TX_TYPE,
};

pub type UpdateId = u32;

/// A high‐level multisig operation that participants can propose.
#[derive(Clone, Debug, Eq, PartialEq, Arbitrary, BorshDeserialize, BorshSerialize)]
pub enum MultisigAction {
    /// Cancel a pending action.
    Cancel(CancelAction),
    /// Propose an update.
    Update(UpdateAction),
}

impl MultisigAction {
    /// Computes a signature hash for this multisig action.
    ///
    /// The hash is computed over the concatenation of:
    /// - The action's Borsh hash (32 bytes)
    /// - The sequence number in big-endian format (8 bytes)
    ///
    /// # Arguments
    /// * `seqno` - Sequence number to include in the hash
    ///
    /// # Returns
    /// A 32-byte hash that can be used for signing
    pub fn compute_sighash(&self, seqno: u64) -> Buf32 {
        let action_hash = compute_borsh_hash(self).0;
        let seqno_bytes = seqno.to_be_bytes();
        let mut data = [0u8; 40];
        data[..32].copy_from_slice(&action_hash);
        data[32..].copy_from_slice(&seqno_bytes);
        hash::raw(&data)
    }

    pub fn tx_type(&self) -> TxType {
        match self {
            MultisigAction::Cancel(_) => CANCEL_TX_TYPE,
            MultisigAction::Update(update) => match update {
                UpdateAction::Multisig(update) => match update.role() {
                    Role::StrataAdministrator => STRATA_ADMIN_MULTISIG_UPDATE_TX_TYPE,
                    Role::StrataSequencerManager => STRATA_SEQ_MANAGER_MULTISIG_UPDATE_TX_TYPE,
                },
                UpdateAction::OperatorSet(_) => OPERATOR_UPDATE_TX_TYPE,
                UpdateAction::Sequencer(_) => SEQUENCER_UPDATE_TX_TYPE,
                UpdateAction::VerifyingKey(vk) => match vk.kind() {
                    ProofType::Asm => ASM_STF_VK_UPDATE_TX_TYPE,
                    ProofType::OlStf => OL_STF_VK_UPDATE_TX_TYPE,
                },
            },
        }
    }
}
