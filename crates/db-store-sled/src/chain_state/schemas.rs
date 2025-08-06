use borsh::{BorshDeserialize, BorshSerialize};
use strata_db::chainstate::*;
use strata_state::{chain_state::Chainstate, state_op::WriteBatch};

use crate::{
    define_table_with_default_codec, define_table_with_integer_key, define_table_without_codec,
    impl_borsh_value_codec,
};

define_table_with_default_codec!(
    /// Table to store write batches.
    (WriteBatchSchema) WriteBatchId => WriteBatch
);

define_table_with_integer_key!(
    /// Table to store state instance data.
    (StateInstanceSchema) StateInstanceId => StateInstanceEntry
);
// TODO: copied over from db-store-rocksdb. needs to go to somewhere common.
/// Describes the entry for a state in the database.
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub(crate) struct StateInstanceEntry {
    pub(crate) toplevel_state: Chainstate,
}

impl StateInstanceEntry {
    pub(crate) fn new(toplevel_state: Chainstate) -> Self {
        Self { toplevel_state }
    }

    pub(crate) fn into_toplevel_state(self) -> Chainstate {
        self.toplevel_state
    }
}
