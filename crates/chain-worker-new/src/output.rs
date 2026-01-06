//! Output types for block execution.

use strata_identifiers::Buf32;
use strata_ol_state_support_types::IndexerWrites;
use strata_ol_state_types::{NativeAccountState, WriteBatch};

/// Output from executing a block with the OL STF.
///
/// This encapsulates all the results from block execution that need to be
/// persisted to the database.
///
/// Note: Logs are not included here because the STF's `verify_block` validates
/// them internally via the `logs_root` commitment in the header.
#[derive(Clone, Debug)]
pub struct OLBlockExecutionOutput {
    /// Computed state root after execution.
    computed_state_root: Buf32,

    /// State changes to persist (the diff).
    write_batch: WriteBatch<NativeAccountState>,

    /// Auxiliary data for indexing (inbox messages, manifests).
    indexer_writes: IndexerWrites,
}

impl OLBlockExecutionOutput {
    /// Creates a new execution output.
    pub fn new(
        computed_state_root: Buf32,
        write_batch: WriteBatch<NativeAccountState>,
        indexer_writes: IndexerWrites,
    ) -> Self {
        Self {
            computed_state_root,
            write_batch,
            indexer_writes,
        }
    }

    /// Returns the computed state root after execution.
    pub fn computed_state_root(&self) -> &Buf32 {
        &self.computed_state_root
    }

    /// Returns the state changes (write batch).
    pub fn write_batch(&self) -> &WriteBatch<NativeAccountState> {
        &self.write_batch
    }

    /// Returns the auxiliary indexer writes.
    pub fn indexer_writes(&self) -> &IndexerWrites {
        &self.indexer_writes
    }

    /// Consumes self and returns the inner components.
    pub fn into_parts(self) -> (Buf32, WriteBatch<NativeAccountState>, IndexerWrites) {
        (
            self.computed_state_root,
            self.write_batch,
            self.indexer_writes,
        )
    }
}

#[cfg(test)]
mod tests {
    use strata_identifiers::{L1BlockId, OLBlockId};
    use strata_ol_state_types::{EpochalState, GlobalState};
    use strata_primitives::{
        epoch::EpochCommitment, l1::L1BlockCommitment, prelude::BitcoinAmount,
    };

    use super::*;

    fn test_epochal_state() -> EpochalState {
        EpochalState::new(
            BitcoinAmount::from_sat(0),
            0,
            L1BlockCommitment::from_height_u64(0, L1BlockId::from(Buf32::zero())).unwrap(),
            EpochCommitment::new(0, 0, OLBlockId::from(Buf32::zero())),
        )
    }

    #[test]
    fn test_output_creation_and_accessors() {
        let state_root = Buf32::from([1u8; 32]);
        let global = GlobalState::new(100);
        let epochal = test_epochal_state();
        let write_batch = WriteBatch::new(global, epochal);
        let indexer_writes = IndexerWrites::new();

        let output = OLBlockExecutionOutput::new(state_root, write_batch, indexer_writes);

        assert_eq!(output.computed_state_root(), &state_root);
        assert!(output.indexer_writes().is_empty());
    }

    #[test]
    fn test_output_into_parts() {
        let state_root = Buf32::from([2u8; 32]);
        let global = GlobalState::new(200);
        let epochal = test_epochal_state();
        let write_batch = WriteBatch::new(global, epochal);
        let indexer_writes = IndexerWrites::new();

        let output = OLBlockExecutionOutput::new(state_root, write_batch, indexer_writes);

        let (root, _batch, writes) = output.into_parts();
        assert_eq!(root, state_root);
        assert!(writes.is_empty());
    }
}
