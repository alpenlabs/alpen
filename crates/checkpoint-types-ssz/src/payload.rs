//! Impl blocks for checkpoint payload types.

use ssz::{Decode, Encode};
use ssz_types::VariableList;
use strata_crypto::hash::raw;
use strata_identifiers::{Buf32, Buf64, Epoch, OLBlockCommitment};
use strata_ol_chain_types_new::OLLog;

use crate::{
    CheckpointPayloadError, CheckpointTip, MAX_PROOF_LEN, OL_DA_DIFF_MAX_SIZE, OUTPUT_MSG_MAX_SIZE,
    ssz_generated::ssz::payload::{CheckpointPayload, CheckpointSidecar, SignedCheckpointPayload},
};

impl CheckpointTip {
    pub fn new(epoch: Epoch, l1_height: u32, l2_commitment: OLBlockCommitment) -> Self {
        Self {
            epoch,
            l1_height,
            l2_commitment,
        }
    }

    pub fn l1_height(&self) -> u32 {
        self.l1_height
    }

    pub fn l2_commitment(&self) -> &OLBlockCommitment {
        &self.l2_commitment
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

    /// Get the state diff bytes.
    pub fn ol_state_diff(&self) -> &[u8] {
        &self.ol_state_diff
    }

    /// Get the OL logs bytes.
    pub fn ol_logs(&self) -> &[u8] {
        &self.ol_logs
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
        new_tip: CheckpointTip,
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
            new_tip,
            sidecar,
            proof,
        })
    }

    pub fn new_tip(&self) -> &CheckpointTip {
        &self.new_tip
    }

    pub fn sidecar(&self) -> &CheckpointSidecar {
        &self.sidecar
    }

    pub fn proof(&self) -> &[u8] {
        &self.proof
    }

    pub fn state_diff_hash(&self) -> Buf32 {
        raw(&self.sidecar.ol_state_diff)
    }

    pub fn ol_logs_hash(&self) -> Buf32 {
        raw(&self.sidecar.ol_logs)
    }

    /// Compute the hash of this payload for signature verification.
    pub fn compute_hash(&self) -> Buf32 {
        raw(&self.as_ssz_bytes())
    }
}

impl SignedCheckpointPayload {
    pub fn new(inner: CheckpointPayload, signature: Buf64) -> Self {
        Self { inner, signature }
    }

    pub fn inner(&self) -> &CheckpointPayload {
        &self.inner
    }

    pub fn signature(&self) -> &Buf64 {
        &self.signature
    }

    /// Verify the signature over the payload hash.
    ///
    /// Returns the payload hash that was signed.
    pub fn payload_hash(&self) -> Buf32 {
        self.inner.compute_hash()
    }
}
