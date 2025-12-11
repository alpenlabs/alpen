//! Checkpoint payload types for L1 posting.

use std::fmt;

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use ssz::Decode;
use strata_identifiers::{
    Buf32, Buf64, Epoch, EpochCommitment, L1BlockCommitment, L2BlockCommitment, L2BlockId, Slot,
    hash::raw,
};
use strata_ol_chain_types_new::OLLog;

// ============================================================================
// L1BlockRange - Range of L1 blocks covered by a checkpoint
// ============================================================================

/// Range of L1 blocks covered by a checkpoint batch.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
pub struct L1BlockRange {
    /// Start of the L1 block range (inclusive).
    pub start: L1BlockCommitment,
    /// End of the L1 block range (inclusive).
    pub end: L1BlockCommitment,
}

impl L1BlockRange {
    /// Creates a new L1 block range.
    pub fn new(start: L1BlockCommitment, end: L1BlockCommitment) -> Self {
        Self { start, end }
    }

    /// Returns the start block commitment.
    pub fn start(&self) -> &L1BlockCommitment {
        &self.start
    }

    /// Returns the end block commitment.
    pub fn end(&self) -> &L1BlockCommitment {
        &self.end
    }
}

// ============================================================================
// L2BlockRange - Range of L2 blocks covered by a checkpoint
// ============================================================================

/// Range of L2 (OL) blocks covered by a checkpoint batch.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
pub struct L2BlockRange {
    /// Start of the L2 block range (inclusive).
    pub start: L2BlockCommitment,
    /// End of the L2 block range (inclusive).
    pub end: L2BlockCommitment,
}

impl L2BlockRange {
    /// Creates a new L2 block range.
    pub fn new(start: L2BlockCommitment, end: L2BlockCommitment) -> Self {
        Self { start, end }
    }

    /// Returns the start block commitment.
    pub fn start(&self) -> &L2BlockCommitment {
        &self.start
    }

    /// Returns the end block commitment.
    pub fn end(&self) -> &L2BlockCommitment {
        &self.end
    }
}

// ============================================================================
// BatchInfo - Metadata about the checkpoint batch
// ============================================================================

/// Contains metadata describing a batch checkpoint, including the L1 and L2 height ranges
/// it covers and the final L2 block ID in that range.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
pub struct BatchInfo {
    /// Checkpoint epoch.
    pub epoch: Epoch,
    /// L1 block range (inclusive) the checkpoint covers.
    pub l1_range: L1BlockRange,
    /// L2 block range (inclusive) the checkpoint covers.
    pub l2_range: L2BlockRange,
}

impl fmt::Display for BatchInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        <Self as fmt::Debug>::fmt(self, f)
    }
}

impl BatchInfo {
    /// Creates new batch info.
    pub fn new(epoch: Epoch, l1_range: L1BlockRange, l2_range: L2BlockRange) -> Self {
        Self {
            epoch,
            l1_range,
            l2_range,
        }
    }

    /// Returns the epoch number.
    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    /// Gets the epoch commitment for this batch.
    pub fn get_epoch_commitment(&self) -> EpochCommitment {
        EpochCommitment::from_terminal(self.epoch, *self.final_l2_block())
    }

    /// Returns the L1 block range.
    pub fn l1_range(&self) -> &L1BlockRange {
        &self.l1_range
    }

    /// Returns the L2 block range.
    pub fn l2_range(&self) -> &L2BlockRange {
        &self.l2_range
    }

    /// Returns the final L1 block commitment in the batch's L1 range.
    pub fn final_l1_block(&self) -> &L1BlockCommitment {
        &self.l1_range.end
    }

    /// Returns the final L2 block commitment in the batch's L2 range.
    pub fn final_l2_block(&self) -> &L2BlockCommitment {
        &self.l2_range.end
    }

    /// Returns the final L2 block ID in the batch's L2 range.
    pub fn final_l2_blockid(&self) -> &L2BlockId {
        self.l2_range.end.blkid()
    }

    /// Check whether the L2 slot is covered by the checkpoint.
    pub fn includes_l2_block(&self, slot: Slot) -> bool {
        slot <= self.l2_range.end.slot()
    }

    /// Check whether the L1 height is covered by the checkpoint.
    pub fn includes_l1_block(&self, height: u64) -> bool {
        height <= self.l1_range.end.height_u64()
    }
}

// ============================================================================
// BatchTransition - State transition info
// ============================================================================

/// Contains transition information in a batch checkpoint, verified by the proof.
///
/// This struct represents a concise summary of the chainstate transition by capturing only the
/// state roots before and after the execution of a batch of blocks.
///
/// # Example
///
/// Given a batch execution transitioning from block `M` to block `N`:
/// - `pre_state_root` represents the chainstate root immediately **before** executing block `M`
///   (i.e., immediately after executing block `M-1`)
/// - `post_state_root` represents the chainstate root immediately **after** executing block `N`
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
pub struct BatchTransition {
    /// Epoch number.
    pub epoch: Epoch,
    /// Chainstate root prior to execution of the batch.
    pub pre_state_root: Buf32,
    /// Chainstate root after batch execution.
    pub post_state_root: Buf32,
}

impl BatchTransition {
    /// Creates a new batch transition.
    pub fn new(epoch: Epoch, pre_state_root: Buf32, post_state_root: Buf32) -> Self {
        Self {
            epoch,
            pre_state_root,
            post_state_root,
        }
    }

    /// Returns the epoch number.
    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    /// Returns the pre-state root.
    pub fn pre_state_root(&self) -> &Buf32 {
        &self.pre_state_root
    }

    /// Returns the post-state root.
    pub fn post_state_root(&self) -> &Buf32 {
        &self.post_state_root
    }
}

// ============================================================================
// CheckpointSidecar - Additional data posted with checkpoint
// ============================================================================

/// Sidecar data posted alongside the checkpoint.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct CheckpointSidecar {
    /// OL state diff blob.
    pub ol_state_diff: Vec<u8>,
    /// OL logs blob (contains withdrawal intents, etc.).
    pub ol_logs: Vec<u8>,
}

impl CheckpointSidecar {
    /// Creates a new checkpoint sidecar.
    pub fn new(ol_state_diff: Vec<u8>, ol_logs: Vec<u8>) -> Self {
        Self {
            ol_state_diff,
            ol_logs,
        }
    }

    /// Returns the OL state diff blob.
    pub fn ol_state_diff(&self) -> &[u8] {
        &self.ol_state_diff
    }

    /// Returns the OL logs blob.
    pub fn ol_logs(&self) -> &[u8] {
        &self.ol_logs
    }

    /// Returns true if the sidecar is empty.
    pub fn is_empty(&self) -> bool {
        self.ol_state_diff.is_empty() && self.ol_logs.is_empty()
    }

    /// Parse and return the OL logs.
    ///
    /// The OL logs are SSZ-serialized `Vec<OLLog>` entries.
    /// Returns `None` if deserialization fails.
    pub fn parse_ol_logs(&self) -> Option<Vec<OLLog>> {
        if self.ol_logs.is_empty() {
            return Some(Vec::new());
        }
        Vec::<OLLog>::from_ssz_bytes(&self.ol_logs).ok()
    }
}

// ============================================================================
// CheckpointCommitment - Core commitment data
// ============================================================================

/// Core commitment data in a checkpoint.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct CheckpointCommitment {
    /// Batch metadata.
    pub batch_info: BatchInfo,
    /// State transition.
    pub transition: BatchTransition,
}

impl CheckpointCommitment {
    /// Creates a new checkpoint commitment.
    pub fn new(batch_info: BatchInfo, transition: BatchTransition) -> Self {
        Self {
            batch_info,
            transition,
        }
    }

    /// Returns the batch info.
    pub fn batch_info(&self) -> &BatchInfo {
        &self.batch_info
    }

    /// Returns the transition.
    pub fn transition(&self) -> &BatchTransition {
        &self.transition
    }
}

// ============================================================================
// CheckpointPayload - The main checkpoint data posted to L1
// ============================================================================

/// Checkpoint payload posted to L1.
///
/// This is the on-chain artifact containing:
/// - Commitment (batch info + transition)
/// - Sidecar (state diff + OL logs)
/// - ZK proof
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct CheckpointPayload {
    /// Core commitment data.
    pub commitment: CheckpointCommitment,
    /// Sidecar with state diff and OL logs.
    pub sidecar: CheckpointSidecar,
    /// ZK proof bytes.
    pub proof: Vec<u8>,
}

impl CheckpointPayload {
    /// Creates a new checkpoint payload.
    pub fn new(
        batch_info: BatchInfo,
        transition: BatchTransition,
        sidecar: CheckpointSidecar,
        proof: Vec<u8>,
    ) -> Self {
        Self {
            commitment: CheckpointCommitment::new(batch_info, transition),
            sidecar,
            proof,
        }
    }

    /// Returns the commitment.
    pub fn commitment(&self) -> &CheckpointCommitment {
        &self.commitment
    }

    /// Returns the batch info.
    pub fn batch_info(&self) -> &BatchInfo {
        &self.commitment.batch_info
    }

    /// Returns the transition.
    pub fn transition(&self) -> &BatchTransition {
        &self.commitment.transition
    }

    /// Returns the sidecar.
    pub fn sidecar(&self) -> &CheckpointSidecar {
        &self.sidecar
    }

    /// Returns the proof bytes.
    pub fn proof(&self) -> &[u8] {
        &self.proof
    }

    /// Returns the epoch number.
    pub fn epoch(&self) -> Epoch {
        self.commitment.batch_info.epoch()
    }

    /// Computes the hash of this payload (for signing).
    pub fn compute_hash(&self) -> Buf32 {
        let encoded = borsh::to_vec(self).expect("borsh serialization should not fail");
        raw(&encoded)
    }
}

// ============================================================================
// SignedCheckpointPayload - Signed checkpoint for L1 posting
// ============================================================================

/// Signed checkpoint payload ready for L1 posting.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct SignedCheckpointPayload {
    /// The checkpoint payload.
    pub inner: CheckpointPayload,
    /// Signature over the payload hash.
    pub signature: Buf64,
}

impl SignedCheckpointPayload {
    /// Creates a new signed checkpoint payload.
    pub fn new(inner: CheckpointPayload, signature: Buf64) -> Self {
        Self { inner, signature }
    }

    /// Returns a reference to the inner checkpoint payload.
    pub fn payload(&self) -> &CheckpointPayload {
        &self.inner
    }

    /// Returns the signature.
    pub fn signature(&self) -> &Buf64 {
        &self.signature
    }

    /// Consumes self and returns the inner payload.
    pub fn into_payload(self) -> CheckpointPayload {
        self.inner
    }
}

impl From<SignedCheckpointPayload> for CheckpointPayload {
    fn from(signed: SignedCheckpointPayload) -> Self {
        signed.inner
    }
}
