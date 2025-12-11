//! Checkpoint claim types for proof verification.

use serde::{Deserialize, Serialize};
use ssz::Encode;
use ssz_derive::{Decode as SszDecode, Encode as SszEncode};
use strata_identifiers::{Buf32, L1BlockCommitment, L2BlockCommitment, hash::raw};
use tree_hash_derive::TreeHash;

use crate::{
    Epoch,
    payload::{CheckpointPayload, L1BlockRange, L2BlockRange},
};

// ============================================================================
// CheckpointClaim - Public input for proof verification
// ============================================================================

/// Checkpoint claim used as public input for proof verification.
///
/// This is reconstructed from:
/// - The checkpoint payload (from L1)
/// - Input messages commitment (computed from L1 manifests)
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, SszEncode, SszDecode, TreeHash)]
pub struct CheckpointClaim {
    /// Epoch number.
    pub epoch: Epoch,
    /// L1 block range.
    pub l1_range: L1BlockRange,
    /// L2 block range.
    pub l2_range: L2BlockRange,
    /// Pre-state root.
    pub pre_state_root: Buf32,
    /// Post-state root.
    pub post_state_root: Buf32,
    /// Hash of the state diff.
    pub state_diff_hash: Buf32,
    /// Commitment to input messages from L1.
    pub input_msgs_commitment: Buf32,
    /// Hash of OL logs.
    pub ol_logs_hash: Buf32,
}

// Borsh compatibility via SSZ (fixed-size, no length prefix)
strata_identifiers::impl_borsh_via_ssz_fixed!(CheckpointClaim);

impl CheckpointClaim {
    /// Creates a new checkpoint claim.
    #[expect(clippy::too_many_arguments, reason = "necessary for full claim data")]
    pub fn new(
        epoch: Epoch,
        l1_range: L1BlockRange,
        l2_range: L2BlockRange,
        pre_state_root: Buf32,
        post_state_root: Buf32,
        state_diff_hash: Buf32,
        input_msgs_commitment: Buf32,
        ol_logs_hash: Buf32,
    ) -> Self {
        Self {
            epoch,
            l1_range,
            l2_range,
            pre_state_root,
            post_state_root,
            state_diff_hash,
            input_msgs_commitment,
            ol_logs_hash,
        }
    }

    /// Returns the epoch number.
    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    /// Returns the L1 block range.
    pub fn l1_range(&self) -> &L1BlockRange {
        &self.l1_range
    }

    /// Returns the L2 block range.
    pub fn l2_range(&self) -> &L2BlockRange {
        &self.l2_range
    }

    /// Returns the pre-state root.
    pub fn pre_state_root(&self) -> &Buf32 {
        &self.pre_state_root
    }

    /// Returns the post-state root.
    pub fn post_state_root(&self) -> &Buf32 {
        &self.post_state_root
    }

    /// Returns the state diff hash.
    pub fn state_diff_hash(&self) -> &Buf32 {
        &self.state_diff_hash
    }

    /// Returns the input messages commitment.
    pub fn input_msgs_commitment(&self) -> &Buf32 {
        &self.input_msgs_commitment
    }

    /// Returns the OL logs hash.
    pub fn ol_logs_hash(&self) -> &Buf32 {
        &self.ol_logs_hash
    }

    /// Serializes the claim to bytes for proof verification.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.as_ssz_bytes()
    }
}

// ============================================================================
// CheckpointClaimBuilder - Builder for constructing claims
// ============================================================================

/// Builder for constructing a [`CheckpointClaim`] from a payload and state data.
///
/// The claim is built using:
/// - "Start" values (pre_state_root, l1_start, l2_start) from checkpoint state
/// - "End" values (post_state_root, l1_range.end, l2_range.end) from the checkpoint payload
/// - Computed hashes from sidecar data
/// - Input messages commitment from auxiliary data
#[derive(Debug)]
pub struct CheckpointClaimBuilder<'a> {
    payload: &'a CheckpointPayload,
    /// Pre-state root from checkpoint state (previous epoch's final_state).
    pre_state_root: Buf32,
    /// Start L1 block from checkpoint state (previous epoch's final L1 block).
    l1_start: L1BlockCommitment,
    /// Start L2 block from checkpoint state (previous epoch's terminal L2 block).
    l2_start: L2BlockCommitment,
    /// Input messages commitment (computed from L1 manifests).
    input_msgs_commitment: Option<Buf32>,
}

impl<'a> CheckpointClaimBuilder<'a> {
    /// Creates a new builder from a checkpoint payload and state values.
    ///
    /// # Arguments
    /// - `payload`: The checkpoint payload from L1
    /// - `pre_state_root`: Previous epoch's final state root (from checkpoint state)
    /// - `l1_start`: Previous epoch's final L1 block (from checkpoint state)
    /// - `l2_start`: Previous epoch's terminal L2 block (from checkpoint state)
    pub fn new(
        payload: &'a CheckpointPayload,
        pre_state_root: Buf32,
        l1_start: L1BlockCommitment,
        l2_start: L2BlockCommitment,
    ) -> Self {
        Self {
            payload,
            pre_state_root,
            l1_start,
            l2_start,
            input_msgs_commitment: None,
        }
    }

    /// Sets the input messages commitment (computed from L1 manifests).
    pub fn with_input_msgs_commitment(mut self, commitment: Buf32) -> Self {
        self.input_msgs_commitment = Some(commitment);
        self
    }

    /// Builds the checkpoint claim.
    ///
    /// Constructs the claim using:
    /// - Start values from state (pre_state_root, l1_start, l2_start)
    /// - End values from payload (post_state_root, l1_range.end, l2_range.end)
    /// - Computed hashes from sidecar data
    pub fn build(self) -> CheckpointClaim {
        let batch_info = self.payload.batch_info();
        let transition = self.payload.transition();
        let sidecar = self.payload.sidecar();

        // Build ranges using start from state, end from payload
        let l1_range = L1BlockRange::new(self.l1_start, *batch_info.l1_range().end());
        let l2_range = L2BlockRange::new(self.l2_start, *batch_info.l2_range().end());

        // Compute hashes for sidecar data
        let state_diff_hash = compute_blob_hash(sidecar.ol_state_diff());
        let ol_logs_hash = compute_blob_hash(sidecar.ol_logs());

        CheckpointClaim {
            epoch: batch_info.epoch(),
            l1_range,
            l2_range,
            pre_state_root: self.pre_state_root,
            post_state_root: *transition.post_state_root(),
            state_diff_hash,
            input_msgs_commitment: self.input_msgs_commitment.unwrap_or_default(),
            ol_logs_hash,
        }
    }
}

/// Computes a hash of a blob (used for state diff and OL logs).
fn compute_blob_hash(data: &[u8]) -> Buf32 {
    raw(data)
}
