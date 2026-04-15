use strata_identifiers::EpochCommitment;
use strata_paas::TaskRecordData;
use zkaleido::ProofReceiptWithMetadata;

use crate::define_table_with_default_codec;

define_table_with_default_codec!(
    /// Checkpoint proofs keyed by the epoch commitment they attest to.
    (CheckpointProofSchema) EpochCommitment => ProofReceiptWithMetadata
);

define_table_with_default_codec!(
    /// Prover task store backing [`strata_paas::TaskStore`].
    ///
    /// Byte-keyed (the key is the serialized `ProofSpec::Task`).
    (ProverTaskTree) Vec<u8> => TaskRecordData
);
