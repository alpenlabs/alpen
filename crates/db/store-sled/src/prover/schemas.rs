use strata_db_types::checkpoint_proof::ProofReceiptEntry;
use strata_identifiers::EpochCommitment;
use strata_paas::TaskRecordData;

use crate::{
    define_table_without_codec, impl_cbor_value_codec, impl_codec_key_codec,
    impl_opaque_value_codec, impl_raw_bytes_key_codec,
};

define_table_without_codec!(
    /// Checkpoint proofs keyed by the epoch commitment they attest to.
    ///
    /// Receipts are stored as opaque bytes; the database does not interpret them.
    (CheckpointProofSchema) EpochCommitment => ProofReceiptEntry
);
impl_codec_key_codec!(CheckpointProofSchema, EpochCommitment);
impl_opaque_value_codec!(CheckpointProofSchema, ProofReceiptEntry);

define_table_without_codec!(
    /// Prover task store backing [`strata_paas::TaskStore`].
    ///
    /// Byte-keyed (the key is the serialized `ProofSpec::Task`), stored verbatim so the
    /// documented kind-tag prefixes sort as written.
    (ProverTaskTree) Vec<u8> => TaskRecordData
);
impl_raw_bytes_key_codec!(ProverTaskTree, Vec<u8>);
impl_cbor_value_codec!(ProverTaskTree, TaskRecordData);
