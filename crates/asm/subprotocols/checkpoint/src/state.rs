use rkyv::{
    Archived, Place, Resolver,
    rancor::Fallible,
    with::{ArchiveWith, DeserializeWith, SerializeWith},
};
use ssz::{Decode, Encode};
use strata_checkpoint_types_ssz::CheckpointTip;
use strata_identifiers::{L1Height, L2BlockCommitment, OLBlockId};
use strata_predicate::{PredicateKey, PredicateKeyBuf};

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

/// Serializer for [`CheckpointTip`] as bytes for rkyv.
struct CheckpointTipAsBytes;

impl ArchiveWith<CheckpointTip> for CheckpointTipAsBytes {
    type Archived = Archived<Vec<u8>>;
    type Resolver = Resolver<Vec<u8>>;

    fn resolve_with(field: &CheckpointTip, resolver: Self::Resolver, out: Place<Self::Archived>) {
        let bytes = field.as_ssz_bytes();
        rkyv::Archive::resolve(&bytes, resolver, out);
    }
}

impl<S> SerializeWith<CheckpointTip, S> for CheckpointTipAsBytes
where
    S: Fallible + ?Sized,
    Vec<u8>: rkyv::Serialize<S>,
{
    fn serialize_with(
        field: &CheckpointTip,
        serializer: &mut S,
    ) -> Result<Self::Resolver, S::Error> {
        let bytes = field.as_ssz_bytes();
        rkyv::Serialize::serialize(&bytes, serializer)
    }
}

impl<D> DeserializeWith<Archived<Vec<u8>>, CheckpointTip, D> for CheckpointTipAsBytes
where
    D: Fallible + ?Sized,
    Archived<Vec<u8>>: rkyv::Deserialize<Vec<u8>, D>,
{
    fn deserialize_with(
        field: &Archived<Vec<u8>>,
        deserializer: &mut D,
    ) -> Result<CheckpointTip, D::Error> {
        let bytes = rkyv::Deserialize::deserialize(field, deserializer)?;
        Ok(CheckpointTip::from_ssz_bytes(&bytes).expect("valid CheckpointTip bytes"))
    }
}

/// Checkpoint subprotocol configuration.
#[derive(Clone, Debug, PartialEq)]
pub struct CheckpointConfig {
    /// Predicate for sequencer signature verification.
    pub sequencer_predicate: PredicateKey,
    /// Predicate for checkpoint ZK proof verification.
    pub checkpoint_predicate: PredicateKey,
    /// Genesis L1 block height.
    pub genesis_l1_height: L1Height,
    /// Genesis OL block ID.
    pub genesis_ol_blkid: OLBlockId,
}

/// Checkpoint subprotocol state.
#[derive(Clone, Debug, PartialEq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct CheckpointState {
    /// Predicate for sequencer signature verification.
    /// Updated via `UpdateSequencerKey` message from admin subprotocol.
    #[rkyv(with = PredicateKeyAsBytes)]
    pub sequencer_predicate: PredicateKey,

    /// Predicate for checkpoint ZK proof verification.
    /// Updated via `UpdateCheckpointPredicate` message from admin subprotocol.
    #[rkyv(with = PredicateKeyAsBytes)]
    pub checkpoint_predicate: PredicateKey,

    /// Last verified checkpoint tip position.
    /// Tracks the OL state that has been proven and verified by ASM.
    #[rkyv(with = CheckpointTipAsBytes)]
    pub verified_tip: CheckpointTip,
}

impl CheckpointState {
    /// Initializes checkpoint state from configuration.
    pub fn init(config: CheckpointConfig) -> Self {
        let genesis_epoch = 0;
        let genesis_l2_slot = 0;
        let genesis_l2_commitment =
            L2BlockCommitment::new(genesis_l2_slot, config.genesis_ol_blkid);
        let genesis_tip = CheckpointTip::new(
            genesis_epoch,
            config.genesis_l1_height,
            genesis_l2_commitment,
        );
        Self::new(
            config.sequencer_predicate,
            config.checkpoint_predicate,
            genesis_tip,
        )
    }

    /// Returns the sequencer predicate for signature verification.
    pub(crate) fn new(
        sequencer_predicate: PredicateKey,
        checkpoint_predicate: PredicateKey,
        verified_tip: CheckpointTip,
    ) -> Self {
        Self {
            sequencer_predicate,
            checkpoint_predicate,
            verified_tip,
        }
    }

    /// Returns the sequencer predicate for signature verification.
    pub fn sequencer_predicate(&self) -> &PredicateKey {
        &self.sequencer_predicate
    }

    /// Returns the checkpoint predicate for proof verification.
    pub fn checkpoint_predicate(&self) -> &PredicateKey {
        &self.checkpoint_predicate
    }

    /// Returns the last verified checkpoint tip.
    pub fn verified_tip(&self) -> &CheckpointTip {
        &self.verified_tip
    }

    /// Update the sequencer predicate with a new Schnorr public key.
    pub(crate) fn update_sequencer_predicate(&mut self, new_predicate: PredicateKey) {
        self.sequencer_predicate = new_predicate
    }

    /// Update the checkpoint predicate.
    pub(crate) fn update_checkpoint_predicate(&mut self, new_predicate: PredicateKey) {
        self.checkpoint_predicate = new_predicate;
    }

    /// Updates the verified checkpoint tip after successful verification.
    pub(crate) fn update_verified_tip(&mut self, new_tip: CheckpointTip) {
        self.verified_tip = new_tip
    }
}
