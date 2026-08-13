use strata_csm_types::ClientUpdateOutput;
use strata_primitives::l1::L1BlockCommitment;

use crate::{define_table_without_codec, impl_bincode_key_codec, impl_cbor_value_codec};

define_table_without_codec!(
    /// Table to store client state updates.
    (ClientUpdateOutputSchema) L1BlockCommitment => ClientUpdateOutput
);
impl_bincode_key_codec!(ClientUpdateOutputSchema, L1BlockCommitment);
impl_cbor_value_codec!(ClientUpdateOutputSchema, ClientUpdateOutput);
