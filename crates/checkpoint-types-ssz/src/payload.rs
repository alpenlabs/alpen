//! Impl blocks for checkpoint payload types.

use ssz_types::VariableList;
use strata_identifiers::{
    Buf64, Epoch, OLBlockCommitment, impl_borsh_via_ssz, impl_borsh_via_ssz_fixed,
};
use strata_ol_chain_types_new::OLLog;

use crate::{
    CheckpointPayload, CheckpointPayloadError, CheckpointSidecar, CheckpointTip,
    MAX_LOG_PAYLOAD_BYTES, MAX_OL_LOGS_PER_CHECKPOINT, MAX_PROOF_LEN, MAX_TOTAL_LOG_PAYLOAD_BYTES,
    OL_DA_DIFF_MAX_SIZE, SignedCheckpointPayload,
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

impl_borsh_via_ssz_fixed!(CheckpointTip);

impl CheckpointSidecar {
    pub fn new(
        ol_state_diff: Vec<u8>,
        ol_logs: Vec<OLLog>,
    ) -> Result<Self, CheckpointPayloadError> {
        let state_diff_len = ol_state_diff.len() as u64;

        let ol_state_diff = VariableList::new(ol_state_diff).map_err(|_| {
            CheckpointPayloadError::StateDiffTooLarge {
                provided: state_diff_len,
                max: OL_DA_DIFF_MAX_SIZE,
            }
        })?;

        validate_ol_logs_payloads(&ol_logs)?;

        let ol_logs_len = ol_logs.len() as u64;
        let ol_logs =
            VariableList::new(ol_logs).map_err(|_| CheckpointPayloadError::OLLogsTooLarge {
                provided: ol_logs_len,
                max: MAX_OL_LOGS_PER_CHECKPOINT,
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
    pub fn ol_logs(&self) -> &[OLLog] {
        &self.ol_logs
    }
}

fn validate_ol_logs_payloads(ol_logs: &[OLLog]) -> Result<(), CheckpointPayloadError> {
    let mut total_payload = 0u64;
    for log in ol_logs {
        let payload_len = log.payload().len() as u64;
        if payload_len > MAX_LOG_PAYLOAD_BYTES as u64 {
            return Err(CheckpointPayloadError::OLLogPayloadTooLarge {
                provided: payload_len,
                max: MAX_LOG_PAYLOAD_BYTES as u64,
            });
        }
        total_payload = total_payload.checked_add(payload_len).ok_or(
            CheckpointPayloadError::OLLogsTotalPayloadTooLarge {
                provided: u64::MAX,
                max: MAX_TOTAL_LOG_PAYLOAD_BYTES as u64,
            },
        )?;
        if total_payload > MAX_TOTAL_LOG_PAYLOAD_BYTES as u64 {
            return Err(CheckpointPayloadError::OLLogsTotalPayloadTooLarge {
                provided: total_payload,
                max: MAX_TOTAL_LOG_PAYLOAD_BYTES as u64,
            });
        }
    }
    Ok(())
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
}

impl_borsh_via_ssz!(CheckpointPayload);

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
}

#[cfg(test)]
mod tests {
    use strata_identifiers::AccountSerial;
    use strata_ol_chain_types_new::OLLog;

    use super::*;

    #[test]
    fn test_checkpoint_sidecar_rejects_oversize_log_payload() {
        let log = OLLog::new(AccountSerial::one(), vec![0u8; MAX_LOG_PAYLOAD_BYTES + 1]);
        let result = CheckpointSidecar::new(vec![], vec![log]);

        assert!(matches!(
            result,
            Err(CheckpointPayloadError::OLLogPayloadTooLarge { .. })
        ));
    }

    #[test]
    fn test_checkpoint_sidecar_rejects_total_log_payload() {
        let mut logs = Vec::new();
        for _ in 0..(MAX_TOTAL_LOG_PAYLOAD_BYTES / MAX_LOG_PAYLOAD_BYTES + 1) {
            logs.push(OLLog::new(
                AccountSerial::one(),
                vec![0u8; MAX_LOG_PAYLOAD_BYTES],
            ));
        }

        let result = CheckpointSidecar::new(vec![], logs);

        assert!(matches!(
            result,
            Err(CheckpointPayloadError::OLLogsTotalPayloadTooLarge { .. })
        ));
    }
}
