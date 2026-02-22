use strata_crypto::hash;
use strata_primitives::buf::Buf32;

use crate::constants::AdminTxType;

/// Defines the sighash computation for a multisig action.
///
/// Each multisig action type implements this trait to provide the data used in
/// computing its signature hash. [`tx_type`](Sighash::tx_type) determines
/// the tag for domain separation,
/// [`sighash_payload`](Sighash::sighash_payload) returns the
/// action-specific bytes included in the hash, and
/// [`compute_sighash`](Sighash::compute_sighash) combines them into a
/// tagged hash.
pub trait Sighash {
    /// Returns the [`AdminTxType`] that identifies this action.
    fn tx_type(&self) -> AdminTxType;

    /// Returns the action-specific payload bytes used in sighash computation.
    fn sighash_payload(&self) -> Vec<u8>;

    /// Returns the precomputed `SHA256(tag)` for this action's sighash tag.
    ///
    /// Defaults to [`AdminTxType::sighash_tag_hash`] derived from
    /// [`tx_type`](Sighash::tx_type).
    fn sighash_tag_hash(&self) -> &'static [u8; 32] {
        self.tx_type().sighash_tag_hash()
    }

    /// Computes a tagged signature hash for this multisig action.
    ///
    /// ```text
    /// SHA256(SHA256(tag) ‖ seqno_be ‖ payload)
    /// ```
    ///
    /// The tag hash is derived from [`AdminTxType`] via
    /// [`sighash_tag_hash`](Sighash::sighash_tag_hash) and provides
    /// domain separation. The fixed-size sequence number precedes the
    /// variable-length payload.
    fn compute_sighash(&self, seqno: u64) -> Buf32 {
        let tag_hash: &[u8] = self.sighash_tag_hash();
        let seqno_bytes = seqno.to_be_bytes();
        let payload = self.sighash_payload();
        hash::sha256_iter([tag_hash, &seqno_bytes, &payload])
    }
}
