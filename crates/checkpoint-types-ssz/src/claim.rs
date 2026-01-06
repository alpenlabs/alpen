//! Impl blocks for checkpoint claim types.

use ssz::Encode;
use ssz_types::FixedBytes;
use strata_identifiers::Epoch;

use crate::{CheckpointScope, ssz_generated::ssz::claim::CheckpointClaim};

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
