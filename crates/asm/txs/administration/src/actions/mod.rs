use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use strata_primitives::hash;

mod cancel;
pub mod updates;

pub use cancel::CancelAction;
use strata_primitives::{buf::Buf32, hash::compute_borsh_hash};
pub use updates::UpdateAction;

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
}
