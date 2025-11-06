use strata_csm_types::ClientUpdateOutput;
use strata_primitives::l1::L1BlockCommitment;

use crate::define_table_with_seek_key_codec;

// Consensus Output Schema and corresponding codecs implementation
define_table_with_seek_key_codec!(
    /// Table to store client state updates.
    (ClientUpdateOutputSchema) L1BlockCommitment => ClientUpdateOutput
);
