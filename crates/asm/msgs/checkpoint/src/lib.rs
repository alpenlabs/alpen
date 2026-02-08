//! Inter-protocol message types for the checkpoint subprotocol.
//!
//! This crate exposes the incoming message enum consumed by checkpoint-v0 so other
//! subprotocols can send configuration updates without depending on the checkpoint
//! implementation crate.

use std::any::Any;

use rkyv::{
    Archived, Place, Resolver,
    rancor::Fallible,
    with::{ArchiveWith, DeserializeWith, SerializeWith},
};
use strata_asm_common::{InterprotoMsg, SubprotocolId};
use strata_asm_proto_checkpoint_txs::CHECKPOINT_V0_SUBPROTOCOL_ID;
use strata_predicate::{PredicateKey, PredicateKeyBuf};
use strata_primitives::buf::Buf32;

/// Incoming messages that the checkpoint v0 subprotocol can receive from other subprotocols.
#[derive(Clone, Debug, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum CheckpointIncomingMsg {
    /// Update the Schnorr public key used to verify sequencer signatures embedded in checkpoints.
    // TODO: (@PG) make this directly take PredicateKey
    UpdateSequencerKey(Buf32),
    /// Update the rollup proving system verifying key used for Groth16 proof verification.
    UpdateCheckpointPredicate(#[rkyv(with = PredicateKeyAsBytes)] PredicateKey),
}

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

impl InterprotoMsg for CheckpointIncomingMsg {
    fn id(&self) -> SubprotocolId {
        CHECKPOINT_V0_SUBPROTOCOL_ID
    }

    fn as_dyn_any(&self) -> &dyn Any {
        self
    }
}
