use strata_primitives::l1::L1BlockCommitment;
use strata_state::{l1::L1BlockId, operation::ClientUpdateOutput};

use crate::{define_table_with_seek_key_codec, define_table_without_codec, impl_borsh_value_codec};

// Consensus Output Schema and corresponding codecs implementation
define_table_with_seek_key_codec!(
    /// Table to store client state updates.
    (ClientUpdateOutputSchema) L1BlockCommitment => ClientUpdateOutput
);
