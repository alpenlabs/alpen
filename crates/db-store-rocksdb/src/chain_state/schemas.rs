use strata_state::state_op::WriteBatchEntry;

use crate::{define_table_with_seek_key_codec, define_table_without_codec, impl_borsh_value_codec};

// Consensus State Schema and corresponding codecs implementation
define_table_with_seek_key_codec!(
    /// Table to store client states.
    (WriteBatchSchema) u64 => WriteBatchEntry
);
