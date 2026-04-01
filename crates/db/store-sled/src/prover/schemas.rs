use strata_db_types::types::{SerializableTaskId, SerializableTaskRecord};
use strata_primitives::proof::{ProofContext, ProofKey};
use zkaleido::ProofReceiptWithMetadata;

use crate::define_table_with_default_codec;

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
