use rkyv::{
    Archive, Archived, Deserialize, Place, Resolver, Serialize,
    rancor::Fallible,
    with::{ArchiveWith, DeserializeWith, SerializeWith},
};
use strata_paas::TaskStatus;
use strata_primitives::proof::{ProofContext, ProofKey};
use typed_sled::codec::{CodecError, KeyCodec, ValueCodec};
use zkaleido::ProofReceiptWithMetadata;

use crate::{
    define_table_with_default_codec, define_table_with_seek_key_codec, define_table_without_codec,
    macros::lexicographic::{LexicographicKey, decode_key, encode_key},
};

/// Serializer for [`TaskStatus`] as JSON bytes for rkyv.
struct TaskStatusAsJson;

impl ArchiveWith<TaskStatus> for TaskStatusAsJson {
    type Archived = Archived<Vec<u8>>;
    type Resolver = Resolver<Vec<u8>>;

    fn resolve_with(field: &TaskStatus, resolver: Self::Resolver, out: Place<Self::Archived>) {
        let bytes = serde_json::to_vec(field).expect("serialize TaskStatus");
        rkyv::Archive::resolve(&bytes, resolver, out);
    }
}

impl<S> SerializeWith<TaskStatus, S> for TaskStatusAsJson
where
    S: Fallible + ?Sized,
    Vec<u8>: rkyv::Serialize<S>,
{
    fn serialize_with(field: &TaskStatus, serializer: &mut S) -> Result<Self::Resolver, S::Error> {
        let bytes = serde_json::to_vec(field).expect("serialize TaskStatus");
        rkyv::Serialize::serialize(&bytes, serializer)
    }
}

impl<D> DeserializeWith<Archived<Vec<u8>>, TaskStatus, D> for TaskStatusAsJson
where
    D: Fallible + ?Sized,
    Archived<Vec<u8>>: rkyv::Deserialize<Vec<u8>, D>,
{
    fn deserialize_with(
        field: &Archived<Vec<u8>>,
        deserializer: &mut D,
    ) -> Result<TaskStatus, D::Error> {
        let bytes = rkyv::Deserialize::deserialize(field, deserializer)?;
        Ok(serde_json::from_slice(&bytes).expect("deserialize TaskStatus"))
    }
}

define_table_without_codec!(
    /// A table to store ProofKey -> ProofReceiptWithMetadata mapping
    (ProofSchema) ProofKey => ProofReceiptWithMetadata
);

impl KeyCodec<ProofSchema> for ProofKey {
    fn encode_key(&self) -> Result<Vec<u8>, CodecError> {
        Ok(encode_key(self))
    }

    fn decode_key(data: &[u8]) -> Result<Self, CodecError> {
        decode_key(data).map_err(|err| CodecError::SerializationFailed {
            schema: ProofSchema::tree_name(),
            source: err.into(),
        })
    }
}

impl ValueCodec<ProofSchema> for ProofReceiptWithMetadata {
    fn encode_value(&self) -> Result<Vec<u8>, CodecError> {
        serde_json::to_vec(self).map_err(|err| CodecError::SerializationFailed {
            schema: ProofSchema::tree_name(),
            source: err.into(),
        })
    }

    fn decode_value(data: &[u8]) -> Result<Self, CodecError> {
        serde_json::from_slice(data).map_err(|err| CodecError::SerializationFailed {
            schema: ProofSchema::tree_name(),
            source: err.into(),
        })
    }
}

define_table_with_seek_key_codec!(
    /// A table to store dependencies of a proof context
    (ProofDepsSchema) ProofContext => Vec<ProofContext>
);

// ============================================================================
// PaaS Task Tracking Schemas
// ============================================================================

/// Serializable task ID for storage
///
/// Uses ProofContext as the program type (what prover-client uses).
/// Backend is stored as u8: 0=Native, 1=SP1, 2=Risc0
#[derive(Debug, Clone, PartialEq, Eq, Hash, Archive, Serialize, Deserialize)]
pub struct SerializableTaskId {
    pub program: ProofContext,
    pub backend: u8,
}

impl LexicographicKey for SerializableTaskId {
    fn encode_lexicographic(&self, out: &mut Vec<u8>) {
        self.program.encode_lexicographic(out);
        self.backend.encode_lexicographic(out);
    }

    fn decode_lexicographic(data: &mut &[u8]) -> anyhow::Result<Self> {
        Ok(Self {
            program: ProofContext::decode_lexicographic(data)?,
            backend: u8::decode_lexicographic(data)?,
        })
    }
}

/// Serializable task record for storage
///
/// Timestamps are stored as seconds since UNIX epoch.
#[derive(Debug, Clone, Archive, Serialize, Deserialize)]
pub struct SerializableTaskRecord {
    pub task_id: SerializableTaskId,
    pub uuid: String,
    #[rkyv(with = TaskStatusAsJson)]
    pub status: TaskStatus,
    pub created_at_secs: u64,
    pub updated_at_secs: u64,
}

define_table_with_seek_key_codec!(
    /// PaaS task storage: TaskId -> TaskRecord
    (PaasTaskTree)
    SerializableTaskId => SerializableTaskRecord
);

define_table_with_default_codec!(
    /// PaaS UUID index: UUID -> TaskId (for reverse lookup)
    (PaasUuidIndexTree)
    String => SerializableTaskId
);
