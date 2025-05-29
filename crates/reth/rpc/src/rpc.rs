use std::sync::Arc;

use alpen_reth_db::{StateDiffProvider, WitnessProvider};
use alpen_reth_statediff::{state::ReconstructedState, BatchStateDiffBuilder, BlockStateDiff};
use jsonrpsee::core::RpcResult;
use revm_primitives::alloy_primitives::B256;
use strata_rpc_utils::{to_jsonrpsee_error, to_jsonrpsee_error_object};

use crate::{BlockWitness, StrataRpcApiServer};

/// rpc implementation
#[derive(Debug, Clone)]
pub struct StrataRPC<DB: Clone + Sized> {
    db: Arc<DB>,
}

impl<DB: Clone + Sized> StrataRPC<DB> {
    /// Create new instance
    pub const fn new(db: Arc<DB>) -> Self {
        Self { db }
    }
}

impl<DB> StrataRpcApiServer for StrataRPC<DB>
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

    fn get_block_state_diff(&self, block_hash: B256) -> RpcResult<Option<BlockStateDiff>> {
        self.db
            .get_state_diff_by_hash(block_hash)
            .map_err(to_jsonrpsee_error("Failed fetching block state diff"))
    }

    fn get_state_root_via_diffs(&self, block_number: u64) -> RpcResult<Option<B256>> {
        let mut builder = BatchStateDiffBuilder::new();

        // First construct the batch state diff consisting of all the changes so far.
        for i in 1..=block_number {
            let block_diff = self
                .db
                .get_state_diff_by_number(i)
                .map_err(to_jsonrpsee_error("Failed fetching block state diff"))?;

            if block_diff.is_none() {
                return RpcResult::Err(to_jsonrpsee_error_object(
                    Some(""),
                    "block diff is missing",
                ));
            }
            builder.apply(block_diff.unwrap());
        }

        // Apply the batch state diff onto the genesis State and return the resulting state root.
        // P.S. currently, genesis is hardcoded to be taken from "dev" spec.
        let mut state = ReconstructedState::new_from_spec("dev")
            .map_err(to_jsonrpsee_error("Can't initialize reconstructured state"))?;
        state
            .apply(builder.build())
            .map_err(to_jsonrpsee_error("Error while reconstructing the state"))?;

        RpcResult::Ok(Some(state.state_root()))
    }
}
