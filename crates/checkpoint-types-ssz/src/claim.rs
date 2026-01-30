//! Impl blocks for checkpoint claim types.

use ssz::Encode;
use ssz_types::FixedBytes;
use strata_identifiers::{Epoch, OLBlockCommitment, impl_borsh_via_ssz, impl_borsh_via_ssz_fixed};

use crate::{L2BlockRange, ssz_generated::ssz::claim::CheckpointClaim};

impl L2BlockRange {
    pub fn new(start: OLBlockCommitment, end: OLBlockCommitment) -> Self {
        Self { start, end }
    }

    pub fn start(&self) -> &OLBlockCommitment {
        &self.start
    }

    pub fn end(&self) -> &OLBlockCommitment {
        &self.end
    }
}

impl_borsh_via_ssz_fixed!(L2BlockRange);

impl CheckpointClaim {
    pub fn new(
        epoch: Epoch,
        l2_range: L2BlockRange,
        asm_manifests_hash: FixedBytes<32>,
        state_diff_hash: FixedBytes<32>,
        ol_logs_hash: FixedBytes<32>,
    ) -> Self {
        Self {
            epoch,
            l2_range,
            asm_manifests_hash,
            state_diff_hash,
            ol_logs_hash,
        }
    }

    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub fn l2_range(&self) -> &L2BlockRange {
        &self.l2_range
    }

    pub fn asm_manifests_hash(&self) -> &FixedBytes<32> {
        &self.asm_manifests_hash
    }

    pub fn state_diff_hash(&self) -> &FixedBytes<32> {
        &self.state_diff_hash
    }

    pub fn ol_logs_hash(&self) -> &FixedBytes<32> {
        &self.ol_logs_hash
    }

    /// Serializes the claim to SSZ bytes for proof verification.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.as_ssz_bytes()
    }
}

impl_borsh_via_ssz!(CheckpointClaim);

#[cfg(test)]
mod tests {
    use strata_test_utils_ssz::ssz_proptest;

    use crate::{
        CheckpointClaim, L2BlockRange,
        test_utils::{checkpoint_claim_strategy, l2_block_range_strategy},
    };

    mod l2_block_range {
        use super::*;
        ssz_proptest!(L2BlockRange, l2_block_range_strategy());
    }

    mod checkpoint_claim {
        use super::*;
        ssz_proptest!(CheckpointClaim, checkpoint_claim_strategy());
    }
}
