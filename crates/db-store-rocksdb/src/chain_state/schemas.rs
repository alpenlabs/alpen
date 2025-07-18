use strata_db::chainstate::*;
use strata_state::state_op::WriteBatch;

use super::types::StateInstanceEntry;
use crate::{define_table_with_seek_key_codec, define_table_without_codec, impl_borsh_value_codec};

define_table_with_seek_key_codec!(
    /// Table to store write batches.
    (WriteBatchSchema) WriteBatchId => WriteBatch
);

define_table_with_seek_key_codec!(
    /// Table to store state instance data.
    (StateInstanceSchema) StateInstanceId => StateInstanceEntry
);
