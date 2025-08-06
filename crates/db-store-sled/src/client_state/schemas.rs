use strata_state::operation::ClientUpdateOutput;

use crate::{define_table_with_integer_key, define_table_without_codec, impl_borsh_value_codec};

define_table_with_integer_key!(
    /// Table to store client state updates.
    (ClientUpdateOutputSchema) u64 => ClientUpdateOutput
);
