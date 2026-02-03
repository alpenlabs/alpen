//! Checkpoint v0 data structures
//!
//! This module defines data structures that maintain compatibility with the current
//! checkpoint implementation while incorporating SPS-62 concepts where applicable.
//!
//! NOTE: This is checkpoint v0 which focuses on feature parity with the current
//! checkpoint system. Future versions will be fully SPS-62 compatible.

use rkyv::{
    Archived, Place, Resolver,
    rancor::Fallible,
    with::{ArchiveWith, DeserializeWith, SerializeWith},
};
use strata_checkpoint_types::Checkpoint;
use strata_identifiers::Epoch;
use strata_predicate::{PredicateKey, PredicateKeyBuf};
use strata_primitives::{block_credential::CredRule, buf::Buf32, l1::L1BlockCommitment};

/// Serializer for [`PredicateKey`] as bytes for rkyv.
struct PredicateKeyAsBytes;

impl ArchiveWith<PredicateKey> for PredicateKeyAsBytes {
    type Archived = Archived<Vec<u8>>;
    type Resolver = Resolver<Vec<u8>>;

    fn resolve_with(field: &PredicateKey, resolver: Self::Resolver, out: Place<Self::Archived>) {
        let bytes = field.as_buf_ref().to_bytes();
        rkyv::Archive::resolve(&bytes, resolver, out);
    }
}

impl<S> SerializeWith<PredicateKey, S> for PredicateKeyAsBytes
where
    S: Fallible + ?Sized,
    Vec<u8>: rkyv::Serialize<S>,
{
    fn serialize_with(
        field: &PredicateKey,
        serializer: &mut S,
    ) -> Result<Self::Resolver, S::Error> {
        let bytes = field.as_buf_ref().to_bytes();
        rkyv::Serialize::serialize(&bytes, serializer)
    }
}

impl<D> DeserializeWith<Archived<Vec<u8>>, PredicateKey, D> for PredicateKeyAsBytes
where
    D: Fallible + ?Sized,
    Archived<Vec<u8>>: rkyv::Deserialize<Vec<u8>, D>,
{
    fn deserialize_with(
        field: &Archived<Vec<u8>>,
        deserializer: &mut D,
    ) -> Result<PredicateKey, D::Error> {
        let bytes = rkyv::Deserialize::deserialize(field, deserializer)?;
        Ok(PredicateKeyBuf::try_from(bytes.as_slice())
            .expect("stored predicate key bytes should be valid")
            .to_owned())
    }
}

/// Checkpoint verifier state for checkpoint v0
///
/// NOTE: This maintains state similar to the current core subprotocol but
/// simplified for checkpoint v0 compatibility
#[derive(Clone, Debug, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct CheckpointV0VerifierState {
    /// The last verified checkpoint
    pub last_checkpoint: Option<Checkpoint>,

    /// Last L1 block where we got a valid checkpoint
    pub last_checkpoint_l1_height: u64,

    /// Current epoch we've verified up to
    pub current_verified_epoch: Epoch,

    /// Credential rule governing signature verification
    pub cred_rule: CredRule,

    /// Predicate used to verify the validity of the checkpoint
    #[rkyv(with = PredicateKeyAsBytes)]
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
            last_checkpoint: None,
            last_checkpoint_l1_height: params.genesis_l1_block.height_u64(),
            current_verified_epoch: 0,
            cred_rule: params.cred_rule.clone(),
            predicate: params.predicate.clone(),
        }
    }

    /// Update state with a newly verified checkpoint
    pub fn update_with_checkpoint(&mut self, checkpoint: Checkpoint, l1_height: u64) {
        let epoch = checkpoint.batch_info().epoch();
        self.last_checkpoint = Some(checkpoint);
        self.last_checkpoint_l1_height = l1_height;
        self.current_verified_epoch = epoch;
    }

    /// Get the latest verified epoch
    pub fn current_epoch(&self) -> Epoch {
        self.current_verified_epoch
    }

    /// Get the epoch value we expect for the next checkpoint.
    pub fn expected_next_epoch(&self) -> Epoch {
        match &self.last_checkpoint {
            Some(_) => self.current_verified_epoch + 1,
            None => 0,
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
