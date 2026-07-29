//! Output types for block execution.

use strata_identifiers::Buf32;
use strata_ol_chain_types::OLLog;
use strata_ol_state_support_types::IndexerWrites;
use strata_ol_state_types::{OLAccountState, WriteBatch};

/// Output from executing a block with the OL STF.
///
/// This encapsulates all the results from block execution that need to be
/// persisted to the database.
#[derive(Clone, Debug)]
pub struct OLBlockExecutionOutput {
    /// Computed state root after execution.
    computed_state_root: Buf32,

    /// State changes to persist (the diff).
    write_batch: WriteBatch<OLAccountState>,

    /// Auxiliary data for indexing (inbox messages, manifests).
    indexer_writes: IndexerWrites,

    /// Logs emitted during execution, in emission order.
    ///
    /// Retained for indexing (e.g. sourcing snark-account update `extra_data`); the STF still
    /// validates them internally via the `logs_root` commitment in the header.
    logs: Vec<OLLog>,
}

impl OLBlockExecutionOutput {
    /// Creates a new execution output.
    pub fn new(
        computed_state_root: Buf32,
        write_batch: WriteBatch<OLAccountState>,
        indexer_writes: IndexerWrites,
        logs: Vec<OLLog>,
    ) -> Self {
        Self {
            computed_state_root,
            write_batch,
            indexer_writes,
            logs,
        }
    }

    /// Returns the computed state root after execution.
    pub fn computed_state_root(&self) -> &Buf32 {
        &self.computed_state_root
    }

    /// Returns the state changes (write batch).
    pub fn write_batch(&self) -> &WriteBatch<OLAccountState> {
        &self.write_batch
    }

    /// Returns the auxiliary indexer writes.
    pub fn indexer_writes(&self) -> &IndexerWrites {
        &self.indexer_writes
    }

    /// Returns the logs emitted during execution, in emission order.
    pub fn logs(&self) -> &[OLLog] {
        &self.logs
    }

    /// Consumes self and returns the inner components.
    pub fn into_parts(self) -> (Buf32, WriteBatch<OLAccountState>, IndexerWrites, Vec<OLLog>) {
        (
            self.computed_state_root,
            self.write_batch,
            self.indexer_writes,
            self.logs,
        )
    }
}

#[cfg(test)]
mod tests {
    use strata_acct_types::AccountSerial;

    use super::*;

    fn sample_logs() -> Vec<OLLog> {
        vec![
            OLLog::new(AccountSerial::from(1u32), vec![1, 2, 3]),
            OLLog::new(AccountSerial::from(2u32), vec![4, 5]),
        ]
    }

    #[test]
    fn test_output_creation_and_accessors() {
        let state_root = Buf32::from([1u8; 32]);
        let write_batch = WriteBatch::default();
        let indexer_writes = IndexerWrites::new();
        let logs = sample_logs();

        let output =
            OLBlockExecutionOutput::new(state_root, write_batch, indexer_writes, logs.clone());

        assert_eq!(output.computed_state_root(), &state_root);
        assert!(output.indexer_writes().is_empty());
        assert_eq!(output.logs(), logs.as_slice());
    }

    #[test]
    fn test_output_into_parts() {
        let state_root = Buf32::from([2u8; 32]);
        let write_batch = WriteBatch::default();
        let indexer_writes = IndexerWrites::new();
        let logs = sample_logs();

        let output =
            OLBlockExecutionOutput::new(state_root, write_batch, indexer_writes, logs.clone());

        let (root, _batch, writes, out_logs) = output.into_parts();
        assert_eq!(root, state_root);
        assert!(writes.is_empty());
        assert_eq!(out_logs, logs);
    }
}
