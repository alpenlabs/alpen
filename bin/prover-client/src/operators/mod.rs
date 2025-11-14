//! A module defining operations for proof generation using ZKVMs.
//!
//! This module provides operators that encapsulate RPC client accessors
//! for fetching data needed for proof generation.
//!
//! NOTE: The original ProvingOp trait and task creation methods have been removed
//! as they are now handled by the PaaS  framework.
//! This module now only contains minimal accessor methods for RPC clients.
//!
//! Supported ZKVMs:
//!
//! - Native
//! - SP1 (requires `sp1` feature enabled)

use std::{future::Future, sync::Arc};

use bitcoind_async_client::Client;
use jsonrpsee::http_client::HttpClient;
use strata_db_store_sled::prover::ProofDBSled;
use strata_params::RollupParams;
use strata_primitives::proof::ProofKey;

use crate::errors::ProvingTaskError;

pub(crate) mod checkpoint;
pub(crate) mod cl_stf;
pub(crate) mod evm_ee;

pub(crate) use checkpoint::CheckpointOperator;
pub(crate) use cl_stf::ClStfOperator;
pub(crate) use evm_ee::EvmEeOperator;

/// Trait for operators that can fetch proof inputs
///
/// This provides a unified interface for all proof operators to fetch
/// the inputs required for proof generation. All operators (Checkpoint,
/// ClStf, EvmEe) implement this trait, establishing a common contract.
pub(crate) trait ProofInputFetcher: Send + Sync {
    /// The type of input this operator fetches
    type Input: Send;

    /// Fetch the input required for proof generation
    ///
    /// # Arguments
    ///
    /// * `task_id` - The proof key identifying what to prove
    /// * `db` - The proof database for retrieving dependencies
    fn fetch_input(
        &self,
        task_id: &ProofKey,
        db: &ProofDBSled,
    ) -> impl Future<Output = Result<Self::Input, ProvingTaskError>> + Send;
}

/// Initialize all proof operators
///
/// Creates and configures the EVM EE, CL STF, and Checkpoint operators
/// with proper dependency injection between them.
///
/// Returns: (CheckpointOperator, ClStfOperator, EvmEeOperator)
pub(crate) fn init_operators(
    _btc_client: Client,
    evm_ee_client: HttpClient,
    cl_client: HttpClient,
    rollup_params: RollupParams,
) -> (CheckpointOperator, ClStfOperator, EvmEeOperator) {
    let rollup_params = Arc::new(rollup_params);

    let evm_ee_operator = EvmEeOperator::new(evm_ee_client.clone());
    let cl_stf_operator = ClStfOperator::new(
        cl_client.clone(),
        Arc::new(evm_ee_operator.clone()),
        rollup_params.clone(),
    );
    let checkpoint_operator =
        CheckpointOperator::new(cl_client.clone(), Arc::new(cl_stf_operator.clone()));

    (checkpoint_operator, cl_stf_operator, evm_ee_operator)
}
