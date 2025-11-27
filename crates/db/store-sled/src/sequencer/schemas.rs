//! Schema definitions for sequencer database tables.

use strata_ol_chain_types::L2BlockId;

use crate::{define_table_with_integer_key, define_table_without_codec, impl_borsh_value_codec};

/// Stored exec payload entry containing the L2 block ID and serialized payload.
#[derive(Debug, Clone, borsh::BorshSerialize, borsh::BorshDeserialize)]
pub(crate) struct ExecPayloadEntry {
    /// The L2 block ID this payload belongs to.
    pub(crate) block_id: L2BlockId,

    /// The serialized EL payload data.
    pub(crate) payload: Vec<u8>,
}

impl ExecPayloadEntry {
    pub(crate) fn new(block_id: L2BlockId, payload: Vec<u8>) -> Self {
        Self { block_id, payload }
    }
}

define_table_with_integer_key!(
    /// A table to store exec payloads by slot number.
    /// Maps slot (u64) to ExecPayloadEntry containing block_id and payload.
    (ExecPayloadSchema) u64 => ExecPayloadEntry
);
