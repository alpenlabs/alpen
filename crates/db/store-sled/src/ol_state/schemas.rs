use strata_identifiers::OLBlockCommitment;
use strata_ol_state_types::{NativeAccountState, OLState, WriteBatch};

use crate::{define_table_without_codec, impl_borsh_key_codec, impl_codec_value_codec};

// Both OLState and WriteBatch use Codec, so we need custom implementations
define_table_without_codec!(
    /// Table to store OLState snapshots keyed by OLBlockCommitment.
    (OLStateSchema) OLBlockCommitment => OLState
);

define_table_without_codec!(
    /// Table to store OL state write batches keyed by OLBlockCommitment.
    (OLWriteBatchSchema) OLBlockCommitment => WriteBatch<NativeAccountState>
);

// Reuse macros for codec implementations
impl_borsh_key_codec!(OLStateSchema, OLBlockCommitment);
impl_codec_value_codec!(OLStateSchema, OLState);

impl_borsh_key_codec!(OLWriteBatchSchema, OLBlockCommitment);
impl_codec_value_codec!(OLWriteBatchSchema, WriteBatch<NativeAccountState>);
