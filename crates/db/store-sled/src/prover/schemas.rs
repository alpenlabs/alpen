use strata_db_types::types::PersistedTaskRecord;
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

define_table_with_default_codec!(
    /// Prover task store backing [`strata_paas::TaskStore`].
    ///
    /// Byte-keyed (the key is the serialized `ProofSpec::Task`).
    (ProverTaskTree) Vec<u8> => PersistedTaskRecord
);
