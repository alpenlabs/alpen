//! Output types for OL block execution.

use strata_identifiers::Hash;
use strata_ol_chain_types_new::OLLog;
use strata_ol_state_support_types::IndexerWrites;
use strata_ol_state_types::{NativeAccountState, WriteBatch};

/// Output from executing an OL block with the new STF.
///
/// This captures all the results of block execution that need to be persisted
/// or used for further processing.
#[derive(Clone, Debug)]
pub struct OLBlockExecutionOutput {
    /// Computed state root after execution.
    computed_state_root: Hash,

    /// Logs emitted during execution.
    logs: Vec<OLLog>,

    /// State changes to persist (account state modifications).
    write_batch: WriteBatch<NativeAccountState>,

    /// Auxiliary data for indexing (inbox messages, manifests, snark updates).
    indexer_writes: IndexerWrites,
}

impl OLBlockExecutionOutput {
    /// Creates a new execution output.
    pub fn new(
        computed_state_root: Hash,
        logs: Vec<OLLog>,
        write_batch: WriteBatch<NativeAccountState>,
        indexer_writes: IndexerWrites,
    ) -> Self {
        Self {
            computed_state_root,
            logs,
            write_batch,
            indexer_writes,
        }
    }

    /// Returns the computed state root after execution.
    pub fn computed_state_root(&self) -> &Hash {
        &self.computed_state_root
    }

    /// Returns the logs emitted during execution.
    pub fn logs(&self) -> &[OLLog] {
        &self.logs
    }

    /// Returns a reference to the write batch (state changes).
    pub fn write_batch(&self) -> &WriteBatch<NativeAccountState> {
        &self.write_batch
    }

    /// Consumes the output and returns the write batch.
    pub fn into_write_batch(self) -> WriteBatch<NativeAccountState> {
        self.write_batch
    }

    /// Returns a reference to the indexer writes.
    pub fn indexer_writes(&self) -> &IndexerWrites {
        &self.indexer_writes
    }

    /// Consumes the output and returns all parts.
    pub fn into_parts(
        self,
    ) -> (
        Hash,
        Vec<OLLog>,
        WriteBatch<NativeAccountState>,
        IndexerWrites,
    ) {
        (
            self.computed_state_root,
            self.logs,
            self.write_batch,
            self.indexer_writes,
        )
    }
}
