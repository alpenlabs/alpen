//! Impl blocks for checkpoint claim types.

use ssz::Encode;
use ssz_types::FixedBytes;
use strata_identifiers::{Epoch, L1Height, OLBlockCommitment};

use crate::{
    CheckpointScope, L1BlockHeightRange, L2BlockRange, ssz_generated::ssz::claim::CheckpointClaim,
};

impl L1BlockHeightRange {
    pub fn new(start: L1Height, end: L1Height) -> Self {
        Self { start, end }
    }

    pub fn start(&self) -> &L1Height {
        &self.start
    }

    pub fn end(&self) -> &L1Height {
        &self.end
    }
}

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

impl CheckpointScope {
    pub fn new(l1_range: L1BlockHeightRange, l2_range: L2BlockRange) -> Self {
        Self { l1_range, l2_range }
    }

    pub fn l1_range(&self) -> &L1BlockHeightRange {
        &self.l1_range
    }

    pub fn l2_range(&self) -> &L2BlockRange {
        &self.l2_range
    }
}

impl CheckpointClaim {
    pub fn new(
        epoch: Epoch,
        scope: CheckpointScope,
        state_diff_hash: FixedBytes<32>,
        input_msgs_commitment: FixedBytes<32>,
        ol_logs_hash: FixedBytes<32>,
    ) -> Self {
        Self {
            epoch,
            scope,
            state_diff_hash,
            input_msgs_commitment,
            ol_logs_hash,
        }
    }

    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub fn scope(&self) -> &CheckpointScope {
        &self.scope
    }

    pub fn state_diff_hash(&self) -> &FixedBytes<32> {
        &self.state_diff_hash
    }

    pub fn input_msgs_commitment(&self) -> &FixedBytes<32> {
        &self.input_msgs_commitment
    }

    pub fn ol_logs_hash(&self) -> &FixedBytes<32> {
        &self.ol_logs_hash
    }

    /// Serializes the claim to SSZ bytes for proof verification.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.as_ssz_bytes()
    }
}
