use strata_state::operation::ClientUpdateOutput;

use crate::{define_table_with_seek_key_codec, define_table_without_codec, impl_borsh_value_codec};

// Consensus Output Schema and corresponding codecs implementation
define_table_with_seek_key_codec!(
    /// Table to store client state updates.
    (ClientUpdateOutputSchema) u64 => ClientUpdateOutput
);
