use strata_state::{client_state::L1ClientState, l1::L1BlockId, operation::ClientUpdateOutput};

use crate::{
    define_table_with_default_codec, define_table_with_seek_key_codec, define_table_without_codec,
    impl_borsh_value_codec,
};

// Consensus Output Schema and corresponding codecs implementation
define_table_with_seek_key_codec!(
    /// Table to store client state updates.
    (ClientUpdateOutputSchema) u64 => ClientUpdateOutput
);

define_table_with_default_codec!(
    /// Table to store client state updates.
    (ClientStateSchema) L1BlockId => L1ClientState
);
