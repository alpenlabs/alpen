use std::sync::Arc;

use alpen_reth_db::{StateDiffProvider, WitnessProvider};
use alpen_reth_statediff::{DaEeStateDiff, DaEeStateDiffSerde, ReconstructedState};
use jsonrpsee::core::RpcResult;
use revm_primitives::alloy_primitives::B256;
use strata_rpc_utils::{to_jsonrpsee_error, to_jsonrpsee_error_object};

use crate::{BlockWitness, StrataRpcApiServer};

/// rpc implementation
#[derive(Debug, Clone)]
pub struct AlpenRPC<DB: Clone + Sized> {
    db: Arc<DB>,
}

impl<DB: Clone + Sized> AlpenRPC<DB> {
    /// Create new instance
    pub fn new(db: Arc<DB>) -> Self {
        Self { db }
    }
}

impl<DB> StrataRpcApiServer for AlpenRPC<DB>
where
    DB: WitnessProvider + StateDiffProvider + Send + Sync + Clone + 'static,
{
    #[doc = "fetch block execution witness data for proving in zkvm"]
    fn get_block_witness(
        &self,
        block_hash: B256,
        json: Option<bool>,
    ) -> RpcResult<Option<BlockWitness>> {
        let res = if json.unwrap_or(false) {
            self.db
                .get_block_witness(block_hash)
                .map(|maybe_witness| maybe_witness.map(BlockWitness::Json))
        } else {
            self.db
                .get_block_witness_raw(block_hash)
                .map(|maybe_witness| maybe_witness.map(BlockWitness::Raw))
        };

        res.map_err(to_jsonrpsee_error("Failed fetching witness"))
    }

    fn get_block_state_diff(&self, block_hash: B256) -> RpcResult<Option<DaEeStateDiffSerde>> {
        let block_diff = self
            .db
            .get_state_diff_by_hash(block_hash)
            .map_err(to_jsonrpsee_error("Failed fetching block state diff"))?;

        // DB now returns DaEeStateDiff directly
        Ok(block_diff.map(|diff| DaEeStateDiffSerde::from(&diff)))
    }

    fn get_state_root_via_diffs(&self, block_number: u64) -> RpcResult<Option<B256>> {
        // Initialize state from genesis
        let mut state = ReconstructedState::new_from_spec("dev")
            .map_err(to_jsonrpsee_error("Can't initialize reconstructed state"))?;

        // Apply each block's diff sequentially
        for i in 1..=block_number {
            let block_diff = self
                .db
                .get_state_diff_by_number(i)
                .map_err(to_jsonrpsee_error("Failed fetching block state diff"))?;

            match block_diff {
                Some(diff) => {
                    state
                        .apply_diff(&diff)
                        .map_err(to_jsonrpsee_error("Error while applying state diff"))?;
                }
                None => {
                    return RpcResult::Err(to_jsonrpsee_error_object(
                        Some("missing_diff"),
                        &format!("state diff missing for block {i}"),
                    ));
                }
            }
        }

        RpcResult::Ok(Some(state.state_root()))
    }

    fn get_batch_state_diff(
        &self,
        from_block: u64,
        to_block: u64,
    ) -> RpcResult<Option<DaEeStateDiffSerde>> {
        if from_block > to_block {
            return RpcResult::Err(to_jsonrpsee_error_object(
                Some("invalid_range"),
                "from_block must be <= to_block",
            ));
        }

        // Aggregate all diffs in the range
        let mut aggregated = DaEeStateDiff::new();

        for i in from_block..=to_block {
            let block_diff = self
                .db
                .get_state_diff_by_number(i)
                .map_err(to_jsonrpsee_error("Failed fetching block state diff"))?;

            match block_diff {
                Some(diff) => aggregated.merge(&diff),
                None => {
                    return RpcResult::Err(to_jsonrpsee_error_object(
                        Some("missing_diff"),
                        &format!("state diff missing for block {i}"),
                    ));
                }
            }
        }

        Ok(Some(DaEeStateDiffSerde::from(&aggregated)))
    }
}
