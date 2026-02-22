use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_params::Role;
use strata_crypto::hash::{compute_borsh_hash, raw};
use strata_l1_txfmt::TagData;

mod cancel;
pub mod updates;

pub use cancel::CancelAction;
use strata_primitives::buf::Buf32;
pub use updates::UpdateAction;

use crate::{
    actions::updates::predicate::ProofType,
    constants::{AdminTxType, ADMINISTRATION_SUBPROTOCOL_ID},
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
        raw(&data)
    }

    pub fn tx_type(&self) -> AdminTxType {
        match self {
            MultisigAction::Cancel(_) => AdminTxType::Cancel,
            MultisigAction::Update(update) => match update {
                UpdateAction::Multisig(update) => match update.role() {
                    Role::StrataAdministrator => AdminTxType::StrataAdminMultisigUpdate,
                    Role::StrataSequencerManager => AdminTxType::StrataSeqManagerMultisigUpdate,
                },
                UpdateAction::OperatorSet(_) => AdminTxType::OperatorUpdate,
                UpdateAction::Sequencer(_) => AdminTxType::SequencerUpdate,
                UpdateAction::VerifyingKey(vk) => match vk.kind() {
                    ProofType::Asm => AdminTxType::AsmStfVkUpdate,
                    ProofType::OLStf => AdminTxType::OlStfVkUpdate,
                },
            },
        }
    }

    /// Constructs the SPS-50 [`TagData`] for this action.
    ///
    /// The tag is built from the administration subprotocol ID and the
    /// action's [`TxType`], with no auxiliary data.
    pub fn tag(&self) -> TagData {
        TagData::new(ADMINISTRATION_SUBPROTOCOL_ID, self.tx_type().into(), vec![])
            .expect("empty aux data always fits")
    }
}
