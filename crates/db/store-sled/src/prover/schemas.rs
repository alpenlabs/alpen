use anyhow::{Error as AnyhowError, anyhow};
use rkyv::{Archive, Deserialize, Serialize, rancor::Error as RkyvError};
use strata_paas::TaskStatus;
use strata_primitives::proof::{ProofContext, ProofKey};
use typed_sled::codec::{CodecError, KeyCodec, ValueCodec};
use zkaleido::{Proof, ProofMetadata, ProofReceipt, ProofReceiptWithMetadata, PublicValues, ZkVm};

use crate::{
    define_table_with_default_codec, define_table_with_seek_key_codec, define_table_without_codec,
    macros::lexicographic::{LexicographicKey, decode_key, encode_key},
};

/// Serializer for [`ProofReceiptWithMetadata`] as bytes for rkyv.
#[derive(Archive, Serialize, Deserialize)]
struct ProofReceiptWithMetadataRkyv {
    receipt: ProofReceiptRkyv,
    metadata: ProofMetadataRkyv,
}

/// Serializer for [`ProofReceipt`] as bytes for rkyv.
#[derive(Archive, Serialize, Deserialize)]
struct ProofReceiptRkyv {
    proof: Vec<u8>,
    public_values: Vec<u8>,
}

/// Serializer for [`ProofMetadata`] as bytes for rkyv.
#[derive(Archive, Serialize, Deserialize)]
struct ProofMetadataRkyv {
    zkvm: u8,
    version: String,
}

impl From<&ProofReceiptWithMetadata> for ProofReceiptWithMetadataRkyv {
    fn from(value: &ProofReceiptWithMetadata) -> Self {
        Self {
            receipt: ProofReceiptRkyv {
                proof: value.receipt().proof().as_bytes().to_vec(),
                public_values: value.receipt().public_values().as_bytes().to_vec(),
            },
            metadata: ProofMetadataRkyv {
                zkvm: zkvm_to_tag(*value.metadata().zkvm()),
                version: value.metadata().version().to_string(),
            },
        }
    }
}

impl TryFrom<ProofReceiptWithMetadataRkyv> for ProofReceiptWithMetadata {
    type Error = anyhow::Error;

    fn try_from(value: ProofReceiptWithMetadataRkyv) -> Result<Self, Self::Error> {
        let zkvm = zkvm_from_tag(value.metadata.zkvm)?;
        let receipt = ProofReceipt::new(
            Proof::new(value.receipt.proof),
            PublicValues::new(value.receipt.public_values),
        );
        let metadata = ProofMetadata::new(zkvm, value.metadata.version);
        Ok(ProofReceiptWithMetadata::new(receipt, metadata))
    }
}

fn zkvm_to_tag(zkvm: ZkVm) -> u8 {
    match zkvm {
        ZkVm::SP1 => 0,
        ZkVm::Risc0 => 1,
        ZkVm::Native => 2,
    }
}

fn zkvm_from_tag(tag: u8) -> Result<ZkVm, anyhow::Error> {
    match tag {
        0 => Ok(ZkVm::SP1),
        1 => Ok(ZkVm::Risc0),
        2 => Ok(ZkVm::Native),
        _ => Err(anyhow!("unknown zkvm tag {tag}")),
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
        let wrapper = ProofReceiptWithMetadataRkyv::from(self);
        rkyv::to_bytes::<RkyvError>(&wrapper)
            .map(|bytes| bytes.as_ref().to_vec())
            .map_err(|err| CodecError::SerializationFailed {
                schema: ProofSchema::tree_name(),
                source: err.into(),
            })
    }

    fn decode_value(data: &[u8]) -> Result<Self, CodecError> {
        let wrapper =
            rkyv::from_bytes::<ProofReceiptWithMetadataRkyv, RkyvError>(data).map_err(|err| {
                CodecError::SerializationFailed {
                    schema: ProofSchema::tree_name(),
                    source: err.into(),
                }
            })?;
        wrapper
            .try_into()
            .map_err(|err: AnyhowError| CodecError::SerializationFailed {
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
