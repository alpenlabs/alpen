//! Predicate key (update VK) rotation message type.

use strata_codec::{Codec, VarVec};

/// Message type ID for snark-account predicate key (update VK) rotations.
///
/// Emitted by the OL STF into the target account's inbox when an admin
/// predicate update is applied, so the execution environment observes the
/// pending rotation at a deterministic position in its inbox ordering. The
/// OL attaches no semantics to its consumption — rotations only activate
/// through the update's own declared predicate. Per the Alpen upgrade
/// design, the EE's policy is to terminate the batch that consumes this
/// message and declare the queued key, making that batch the last one
/// proven under the old VK.
pub const PREDICATE_UPDATE_MSG_TYPE_ID: u16 = 0x20;

/// Max length of the raw predicate key bytes carried by [`PredicateUpdateMsgData`].
///
/// Chosen independently of `strata_predicate::MAX_CONDITION_LEN` since this crate
/// deliberately does not depend on `strata-predicate`; it's sized generously above the
/// realistic max (a 1-byte type id plus a 1024-byte condition).
pub const MAX_PREDICATE_KEY_BYTES: u32 = 2048;

/// Bounded byte vec holding the predicate key's raw serialized form.
pub type PredicateKeyBufVec = VarVec<u8, MAX_PREDICATE_KEY_BYTES>;

/// Message data for a snark-account predicate key (update VK) rotation.
///
/// Carries the new predicate key in its raw serialized form, `[id: u8][condition:
/// bytes...]` (as produced by `strata_predicate::PredicateKeyBuf::to_bytes`), rather than
/// the typed `PredicateKey` itself — this crate deliberately does not depend on
/// `strata-predicate`.
#[derive(Debug, Clone, PartialEq, Eq, Codec)]
pub struct PredicateUpdateMsgData {
    predicate_key: PredicateKeyBufVec,
}

impl PredicateUpdateMsgData {
    /// Creates a new instance from the predicate key's raw serialized bytes.
    ///
    /// Returns `None` if the bytes exceed [`MAX_PREDICATE_KEY_BYTES`].
    pub fn new(predicate_key: Vec<u8>) -> Option<Self> {
        Some(Self {
            predicate_key: PredicateKeyBufVec::from_vec(predicate_key)?,
        })
    }

    /// Gets the predicate key's raw serialized bytes.
    pub fn predicate_key_bytes(&self) -> &[u8] {
        self.predicate_key.as_slice()
    }
}

#[cfg(test)]
mod tests {
    use strata_codec::{decode_buf_exact, encode_to_vec};

    use super::*;

    #[test]
    fn test_predicate_update_msg_data_codec_round_trip() {
        let msg_data = PredicateUpdateMsgData::new(vec![10u8, 1, 2, 3, 4])
            .expect("predicate key bytes should fit");

        let encoded = encode_to_vec(&msg_data).expect("encoding should succeed");
        let decoded: PredicateUpdateMsgData =
            decode_buf_exact(&encoded).expect("decoding should succeed");

        assert_eq!(decoded, msg_data);
    }

    #[test]
    fn test_predicate_update_msg_data_rejects_oversize_key() {
        let oversize = vec![0u8; MAX_PREDICATE_KEY_BYTES as usize + 1];
        assert!(PredicateUpdateMsgData::new(oversize).is_none());
    }
}
