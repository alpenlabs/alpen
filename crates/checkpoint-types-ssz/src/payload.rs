//! Impl blocks for checkpoint payload types.

use ssz_types::VariableList;
use strata_identifiers::{
    Buf64, Epoch, OLBlockCommitment, impl_borsh_via_ssz, impl_borsh_via_ssz_fixed,
};
use strata_ol_chain_types_new::OLLog;

use crate::{
    CheckpointPayload, CheckpointPayloadError, CheckpointSidecar, CheckpointTip,
    MAX_OL_LOGS_PER_CHECKPOINT, MAX_PROOF_LEN, OL_DA_DIFF_MAX_SIZE, SignedCheckpointPayload,
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

        let ol_logs_len = ol_logs.len() as u64;
        let ol_logs =
            VariableList::new(ol_logs).map_err(|_| CheckpointPayloadError::OlLogsTooLarge {
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

impl_borsh_via_ssz!(CheckpointSidecar);

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

impl_borsh_via_ssz!(SignedCheckpointPayload);

#[cfg(any(test, feature = "test-utils"))]
impl<'a> arbitrary::Arbitrary<'a> for CheckpointTip {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(Self {
            epoch: u.arbitrary()?,
            l1_height: u.arbitrary()?,
            l2_commitment: u.arbitrary()?,
        })
    }
}

#[cfg(any(test, feature = "test-utils"))]
impl<'a> arbitrary::Arbitrary<'a> for CheckpointSidecar {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let state_diff_len = u.int_in_range(0..=1024)?;
        let state_diff: Vec<u8> = (0..state_diff_len)
            .map(|_| u.arbitrary())
            .collect::<arbitrary::Result<_>>()?;

        let logs_len = u.int_in_range(0..=10)?;
        let ol_logs: Vec<OLLog> = (0..logs_len)
            .map(|_| u.arbitrary())
            .collect::<arbitrary::Result<_>>()?;

        Self::new(state_diff, ol_logs).map_err(|_| arbitrary::Error::IncorrectFormat)
    }
}

#[cfg(any(test, feature = "test-utils"))]
impl<'a> arbitrary::Arbitrary<'a> for CheckpointPayload {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let proof_len = u.int_in_range(0..=512)?;
        let proof: Vec<u8> = (0..proof_len)
            .map(|_| u.arbitrary())
            .collect::<arbitrary::Result<_>>()?;

        Self::new(u.arbitrary()?, u.arbitrary()?, proof)
            .map_err(|_| arbitrary::Error::IncorrectFormat)
    }
}

#[cfg(any(test, feature = "test-utils"))]
impl<'a> arbitrary::Arbitrary<'a> for SignedCheckpointPayload {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(Self {
            inner: u.arbitrary()?,
            signature: u.arbitrary()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use strata_test_utils_ssz::ssz_proptest;

    use crate::{
        CheckpointPayload, CheckpointSidecar, CheckpointTip, SignedCheckpointPayload,
        test_utils::{
            checkpoint_payload_strategy, checkpoint_sidecar_strategy, checkpoint_tip_strategy,
            signed_checkpoint_payload_strategy,
        },
    };

    mod checkpoint_tip {
        use super::*;
        ssz_proptest!(CheckpointTip, checkpoint_tip_strategy());
    }

    mod checkpoint_sidecar {
        use super::*;
        ssz_proptest!(CheckpointSidecar, checkpoint_sidecar_strategy());
    }

    mod checkpoint_payload {
        use super::*;
        ssz_proptest!(CheckpointPayload, checkpoint_payload_strategy());
    }

    mod signed_checkpoint_payload {
        use super::*;
        ssz_proptest!(
            SignedCheckpointPayload,
            signed_checkpoint_payload_strategy()
        );
    }
}
