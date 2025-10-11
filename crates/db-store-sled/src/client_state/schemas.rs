use strata_csm_types::ClientUpdateOutput;
use strata_primitives::l1::L1BlockCommitment;

use crate::{define_table_with_default_codec, define_table_without_codec, impl_borsh_value_codec};

define_table_with_default_codec!(
    /// Table to store client state updates.
    (ClientUpdateOutputSchema) L1BlockCommitment => ClientUpdateOutput
);
