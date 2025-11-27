use strata_paas::TaskStatus;
use strata_primitives::proof::{ProofContext, ProofKey};
use zkaleido::ProofReceiptWithMetadata;

use crate::{define_table_with_default_codec, define_table_without_codec, impl_borsh_value_codec};

define_table_with_default_codec!(
    /// A table to store ProofKey -> ProofReceiptWithMetadata mapping
    (ProofSchema) ProofKey => ProofReceiptWithMetadata
);

define_table_with_default_codec!(
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
#[derive(Debug, Clone, PartialEq, Eq, Hash, borsh::BorshSerialize, borsh::BorshDeserialize)]
pub struct SerializableTaskId {
    pub program: ProofContext,
    pub backend: u8,
}

/// Serializable task record for storage
///
/// Timestamps are stored as seconds since UNIX epoch.
#[derive(Debug, Clone, borsh::BorshSerialize, borsh::BorshDeserialize)]
pub struct SerializableTaskRecord {
    pub task_id: SerializableTaskId,
    pub uuid: String,
    pub status: TaskStatus,
    pub created_at_secs: u64,
    pub updated_at_secs: u64,
}

define_table_with_default_codec!(
    /// PaaS task storage: TaskId -> TaskRecord
    (PaasTaskTree)
    SerializableTaskId => SerializableTaskRecord
);

define_table_with_default_codec!(
    /// PaaS UUID index: UUID -> TaskId (for reverse lookup)
    (PaasUuidIndexTree)
    String => SerializableTaskId
);
