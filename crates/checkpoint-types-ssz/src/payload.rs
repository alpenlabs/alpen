//! Impl blocks for checkpoint payload types.

use ssz::{Decode, Encode};
use ssz_types::VariableList;
use strata_identifiers::{Buf32, Buf64, Epoch, OLBlockCommitment, hash};
use strata_ol_chain_types_new::OLLog;

use crate::{
    CheckpointPayloadError, L1Commitment, MAX_PROOF_LEN, OL_DA_DIFF_MAX_SIZE, OUTPUT_MSG_MAX_SIZE,
    ssz_generated::ssz::payload::{
        BatchInfo, BatchTransition, CheckpointCommitment, CheckpointPayload, CheckpointSidecar,
        L1BlockRange, L2BlockRange, SignedCheckpointPayload,
    },
};

impl L1BlockRange {
    pub fn new(start: L1Commitment, end: L1Commitment) -> Self {
        Self { start, end }
    }
}

impl L2BlockRange {
    pub fn new(start: OLBlockCommitment, end: OLBlockCommitment) -> Self {
        Self { start, end }
    }
}

impl BatchInfo {
    pub fn new(epoch: Epoch, l1_range: L1BlockRange, l2_range: L2BlockRange) -> Self {
        Self {
            epoch,
            l1_range,
            l2_range,
        }
    }
}

impl BatchTransition {
    pub fn new(pre_state_root: Buf32, post_state_root: Buf32) -> Self {
        Self {
            pre_state_root,
            post_state_root,
        }
    }
}

impl CheckpointCommitment {
    pub fn new(batch_info: BatchInfo, transition: BatchTransition) -> Self {
        Self {
            batch_info,
            transition,
        }
    }

    pub fn epoch(&self) -> Epoch {
        self.batch_info.epoch
    }
}

impl CheckpointSidecar {
    pub fn new(ol_state_diff: Vec<u8>, ol_logs: Vec<u8>) -> Result<Self, CheckpointPayloadError> {
        let state_diff_len = ol_state_diff.len() as u64;
        let ol_logs_len = ol_logs.len() as u64;

        let ol_state_diff = VariableList::new(ol_state_diff).map_err(|_| {
            CheckpointPayloadError::StateDiffTooLarge {
                provided: state_diff_len,
                max: OL_DA_DIFF_MAX_SIZE,
            }
        })?;
        let ol_logs =
            VariableList::new(ol_logs).map_err(|_| CheckpointPayloadError::OlLogsTooLarge {
                provided: ol_logs_len,
                max: OUTPUT_MSG_MAX_SIZE,
            })?;
        Ok(Self {
            ol_state_diff,
            ol_logs,
        })
    }

    /// Parse the OL logs from the sidecar bytes.
    ///
    /// Returns `None` if the logs cannot be decoded.
    pub fn parse_ol_logs(&self) -> Option<Vec<OLLog>> {
        if self.ol_logs.is_empty() {
            return Some(Vec::new());
        }
        Vec::<OLLog>::from_ssz_bytes(&self.ol_logs).ok()
    }
}

impl CheckpointPayload {
    pub fn new(
        commitment: CheckpointCommitment,
        sidecar: CheckpointSidecar,
        proof: Vec<u8>,
    ) -> Result<Self, CheckpointPayloadError> {
        let proof_len = proof.len() as u64;
        let proof =
            VariableList::new(proof).map_err(|_| CheckpointPayloadError::ProofTooLarge {
                provided: proof_len,
                max: MAX_PROOF_LEN,
            })?;
        Ok(Self {
            commitment,
            sidecar,
            proof,
        })
    }

    pub fn epoch(&self) -> Epoch {
        self.commitment.epoch()
    }

    pub fn state_diff_hash(&self) -> Buf32 {
        hash::raw(&self.sidecar.ol_state_diff)
    }

    pub fn ol_logs_hash(&self) -> Buf32 {
        hash::raw(&self.sidecar.ol_logs)
    }

    /// Compute the hash of this payload for signature verification.
    pub fn compute_hash(&self) -> Buf32 {
        hash::raw(&self.as_ssz_bytes())
    }
}

impl SignedCheckpointPayload {
    pub fn new(inner: CheckpointPayload, signature: Buf64) -> Self {
        Self { inner, signature }
    }
}
