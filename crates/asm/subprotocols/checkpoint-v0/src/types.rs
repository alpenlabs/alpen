//! Checkpoint v0 data structures
//!
//! This module defines data structures that maintain compatibility with the current
//! checkpoint implementation while incorporating SPS-62 concepts where applicable.
//!
//! NOTE: This is checkpoint v0 which focuses on feature parity with the current
//! checkpoint system. Future versions will be fully SPS-62 compatible.

use borsh::{BorshDeserialize, BorshSerialize};
use ssz_derive::{Decode, Encode};
use strata_identifiers::Epoch;
use strata_params::CredRule;
use strata_predicate::PredicateKey;
use strata_primitives::{L1Height, buf::Buf32, l1::L1BlockCommitment};

/// Checkpoint verifier state for checkpoint v0
///
/// NOTE: This maintains state similar to the current core subprotocol but
/// simplified for checkpoint v0 compatibility
#[derive(Clone, Debug, BorshSerialize, BorshDeserialize, Encode, Decode)]
pub struct CheckpointV0VerifierState {
    /// Whether we have accepted at least one checkpoint.
    pub has_verified_checkpoint: bool,

    /// Last L1 block where we got a valid checkpoint
    pub last_checkpoint_l1_height: L1Height,

    /// Current epoch we've verified up to
    pub current_verified_epoch: Epoch,

    /// Credential rule governing signature verification
    #[ssz(with = "cred_rule_ssz")]
    pub cred_rule: CredRule,

    /// Predicate used to verify the validity of the checkpoint
    pub predicate: PredicateKey,
}

/// Verification parameters for checkpoint v0
///
/// NOTE: This bridges to the current verification system while maintaining
/// some SPS-62 concepts for future compatibility.
/// Configuration parameters don't need serialization - they're provided at init.
#[derive(Clone, Debug)]
pub struct CheckpointV0VerificationParams {
    /// Genesis L1 block commitment
    pub genesis_l1_block: L1BlockCommitment,

    /// Credential rule governing signature verification
    pub cred_rule: CredRule,

    /// Predicate used to verify the validity of the checkpoint
    pub predicate: PredicateKey,
}

/// Compatibility functions for working with current checkpoint types
impl CheckpointV0VerifierState {
    /// Initialize from genesis parameters
    pub fn new(params: &CheckpointV0VerificationParams) -> Self {
        Self {
            has_verified_checkpoint: false,
            last_checkpoint_l1_height: params.genesis_l1_block.height(),
            current_verified_epoch: 0,
            cred_rule: params.cred_rule.clone(),
            predicate: params.predicate.clone(),
        }
    }

    /// Update state with a newly verified checkpoint
    pub fn update_with_checkpoint(&mut self, epoch: Epoch, l1_height: L1Height) {
        self.has_verified_checkpoint = true;
        self.last_checkpoint_l1_height = l1_height;
        self.current_verified_epoch = epoch;
    }

    /// Get the latest verified epoch
    pub fn current_epoch(&self) -> Epoch {
        self.current_verified_epoch
    }

    /// Get the epoch value we expect for the next checkpoint.
    pub fn expected_next_epoch(&self) -> Epoch {
        match self.has_verified_checkpoint {
            true => self.current_verified_epoch + 1,
            false => 0,
        }
    }

    /// Check if we can accept a checkpoint for the given epoch
    ///
    /// Returns `true` if the epoch is exactly one greater than the current verified epoch.
    /// This enforces sequential epoch progression without gaps.
    ///
    /// # Arguments
    /// * `epoch` - The epoch number to validate
    ///
    /// # Returns
    /// `true` if the epoch can be accepted, `false` otherwise
    pub fn can_accept_epoch(&self, epoch: Epoch) -> bool {
        epoch == self.expected_next_epoch()
    }

    /// Update the sequencer public key used to validate checkpoint signatures.
    pub fn update_sequencer_key(&mut self, new_pubkey: Buf32) {
        self.cred_rule = CredRule::SchnorrKey(new_pubkey);
    }

    /// Update the rollup verifying key used for proof verification.
    pub fn update_predicate(&mut self, new_predicate: PredicateKey) {
        self.predicate = new_predicate;
    }
}

#[expect(unreachable_pub, reason = "used by ssz_derive field adapters")]
mod cred_rule_ssz {
    pub mod encode {
        use borsh::to_vec;
        use ssz::Encode as SszEncode;
        use strata_params::CredRule;

        pub fn is_ssz_fixed_len() -> bool {
            <Vec<u8> as SszEncode>::is_ssz_fixed_len()
        }

        pub fn ssz_fixed_len() -> usize {
            <Vec<u8> as SszEncode>::ssz_fixed_len()
        }

        pub fn ssz_bytes_len(value: &CredRule) -> usize {
            to_vec(value)
                .expect("CredRule borsh encoding should not fail")
                .ssz_bytes_len()
        }

        pub fn ssz_append(value: &CredRule, buf: &mut Vec<u8>) {
            to_vec(value)
                .expect("CredRule borsh encoding should not fail")
                .ssz_append(buf);
        }
    }

    pub mod decode {
        use borsh::from_slice;
        use ssz::{Decode as SszDecode, DecodeError};
        use strata_params::CredRule;

        pub fn is_ssz_fixed_len() -> bool {
            <Vec<u8> as SszDecode>::is_ssz_fixed_len()
        }

        pub fn ssz_fixed_len() -> usize {
            <Vec<u8> as SszDecode>::ssz_fixed_len()
        }

        pub fn from_ssz_bytes(bytes: &[u8]) -> Result<CredRule, DecodeError> {
            let encoded = Vec::<u8>::from_ssz_bytes(bytes)?;
            from_slice(&encoded).map_err(|err| {
                DecodeError::BytesInvalid(format!("invalid CredRule payload: {err}"))
            })
        }
    }
}
