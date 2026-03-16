//! Impl blocks for checkpoint payload types.

use ssz_primitives::FixedBytes;
use ssz_types::VariableList;
use strata_identifiers::{
    Buf32, Epoch, OLBlockCommitment, OLBlockId, impl_borsh_via_ssz, impl_borsh_via_ssz_fixed,
};
use strata_ol_chain_types_new::{OLBlockHeader, OLLog};
use tree_hash::{Sha256Hasher, TreeHash};

use crate::{
    CheckpointPayload, CheckpointPayloadError, CheckpointSidecar, CheckpointTip,
    MAX_LOG_PAYLOAD_BYTES, MAX_OL_LOGS_PER_CHECKPOINT, MAX_PROOF_LEN, MAX_TOTAL_LOG_PAYLOAD_BYTES,
    OL_DA_DIFF_MAX_SIZE, TerminalHeaderComplement,
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

/// Minimal subset of the terminal [`OLBlockHeader`] for L1 reconstruction.
///
/// A fresh sequencer can reconstruct OL state from L1 but cannot recover the
/// terminal header needed to continue block production. Most header fields are
/// derivable (`slot`/`blkid` from `new_tip.l2_commitment`, `epoch` from
/// `new_tip.epoch`, `state_root` from DA + manifest reconstruction, `is_terminal`
/// by checkpoint semantics), but these four are not available from L1 data.
///
/// The proof commits to the SSZ tree hash root of [`TerminalHeaderComplement`] in
/// [`crate::CheckpointClaim`], so the L1 verifier can enforce sidecar integrity
/// without a full header preimage.
impl TerminalHeaderComplement {
    pub fn new(
        timestamp: u64,
        parent_blkid: OLBlockId,
        body_root: Buf32,
        logs_root: Buf32,
    ) -> Self {
        Self {
            timestamp,
            parent_blkid,
            body_root,
            logs_root,
        }
    }

    /// Constructs [`TerminalHeaderComplement`] from a full [`OLBlockHeader`].
    pub fn from_full_header(header: &OLBlockHeader) -> Self {
        Self::new(
            header.timestamp(),
            *header.parent_blkid(),
            *header.body_root(),
            *header.logs_root(),
        )
    }

    pub fn timestamp(&self) -> u64 {
        self.timestamp
    }

    pub fn parent_blkid(&self) -> &OLBlockId {
        &self.parent_blkid
    }

    pub fn body_root(&self) -> &Buf32 {
        &self.body_root
    }

    pub fn logs_root(&self) -> &Buf32 {
        &self.logs_root
    }

    /// Computes the SSZ tree hash root of this complement.
    pub fn compute_hash(&self) -> FixedBytes<32> {
        FixedBytes::<32>::from(TreeHash::<Sha256Hasher>::tree_hash_root(self).0)
    }
}

impl CheckpointSidecar {
    pub fn new(
        ol_state_diff: Vec<u8>,
        ol_logs: Vec<OLLog>,
        terminal_header_complement: TerminalHeaderComplement,
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
            terminal_header_complement,
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

    /// Get the terminal header subset.
    pub fn terminal_header_complement(&self) -> &TerminalHeaderComplement {
        &self.terminal_header_complement
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

#[cfg(test)]
mod tests {
    use strata_identifiers::{AccountSerial, Buf32, OLBlockId};
    use strata_ol_chain_types_new::OLLog;

    use super::*;

    fn default_terminal_header_complement() -> TerminalHeaderComplement {
        TerminalHeaderComplement::new(0, OLBlockId::null(), Buf32::zero(), Buf32::zero())
    }

    #[test]
    fn test_checkpoint_sidecar_rejects_oversize_log_payload() {
        let log = OLLog::new(AccountSerial::one(), vec![0u8; MAX_LOG_PAYLOAD_BYTES + 1]);
        let result =
            CheckpointSidecar::new(vec![], vec![log], default_terminal_header_complement());

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

        let result = CheckpointSidecar::new(vec![], logs, default_terminal_header_complement());

        assert!(matches!(
            result,
            Err(CheckpointPayloadError::OLLogsTotalPayloadTooLarge { .. })
        ));
    }
}
