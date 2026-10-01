use strata_identifiers::OLBlockCommitment;
use strata_ol_state_container::OLStateContainer;
use strata_ol_state_types_v1::WriteBatch;

use crate::{
    define_table_without_codec, impl_cbor_value_codec, impl_codec_key_codec, impl_codec_value_codec,
};

// OLStateContainer uses CBOR, WriteBatch uses Codec
define_table_without_codec!(
    /// Table to store OL state snapshots keyed by OLBlockCommitment.
    (OLStateSchema) OLBlockCommitment => OLStateContainer
);

define_table_without_codec!(
    /// Table to store OL state write batches keyed by OLBlockCommitment.
    (OLWriteBatchSchema) OLBlockCommitment => WriteBatch
);

// OLBlockCommitment uses Codec for key encoding (big-endian for proper linear scans)
impl_codec_key_codec!(OLStateSchema, OLBlockCommitment);
impl_codec_key_codec!(OLWriteBatchSchema, OLBlockCommitment);

// Snapshots use the container's serde form. Decoding checks the chainstate
// against its committed root and rejects unsupported specs, so a corrupt or
// legacy (bare `OLStateV1` SSZ) value fails instead of loading with defaulted
// versions.
impl_cbor_value_codec!(OLStateSchema, OLStateContainer);

// WriteBatch uses Codec trait (contains non-SSZ types like BTreeMap, SerialMap)
impl_codec_value_codec!(OLWriteBatchSchema, WriteBatch);
