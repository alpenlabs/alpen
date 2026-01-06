//! Impl blocks for checkpoint claim types.

use ssz::Encode;
use strata_identifiers::{Buf32, Epoch};

use crate::{
    BatchTransition, CheckpointPayload, L1BlockRange, L2BlockRange,
    ssz_generated::ssz::claim::CheckpointClaim,
};

impl CheckpointClaim {
    pub fn new(
        epoch: Epoch,
        l1_range: L1BlockRange,
        l2_range: L2BlockRange,
        transition: BatchTransition,
        state_diff_hash: Buf32,
        input_msgs_commitment: Buf32,
        ol_logs_hash: Buf32,
    ) -> Self {
        Self {
            epoch,
            l1_range,
            l2_range,
            transition,
            state_diff_hash,
            input_msgs_commitment,
            ol_logs_hash,
        }
    }

    /// Create a claim from a checkpoint payload and auxiliary data.
    ///
    /// The `pre_state_root` comes from local state (previous epoch's final state).
    /// The `input_msgs_commitment` is a rolling hash of ASM manifest hashes.
    pub fn from_payload(
        payload: &CheckpointPayload,
        pre_state_root: Buf32,
        input_msgs_commitment: Buf32,
    ) -> Self {
        let batch_info = &payload.commitment.batch_info;
        let post_state_root = payload.commitment.post_state_root;

        Self {
            epoch: batch_info.epoch,
            l1_range: batch_info.l1_range,
            l2_range: batch_info.l2_range,
            transition: BatchTransition::new(pre_state_root, post_state_root),
            state_diff_hash: payload.state_diff_hash(),
            input_msgs_commitment,
            ol_logs_hash: payload.ol_logs_hash(),
        }
    }

    /// Serializes the claim to SSZ bytes for proof verification.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.as_ssz_bytes()
    }
}
