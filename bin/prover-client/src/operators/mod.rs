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

use std::future::Future;

use jsonrpsee::http_client::HttpClient;
use strata_db_store_sled::prover::ProofDBSled;
use strata_primitives::proof::ProofKey;

use crate::errors::ProvingTaskError;

pub(crate) mod checkpoint;
pub(crate) mod evm_ee;

pub(crate) use checkpoint::CheckpointOperator;
pub(crate) use evm_ee::EvmEeOperator;

/// Trait for operators that can fetch proof inputs
///
/// This provides a unified interface for all proof operators to fetch
/// the inputs required for proof generation. All operators (Checkpoint,
/// EvmEe) implement this trait, establishing a common contract.
pub(crate) trait ProofInputFetcher: Send + Sync {
    /// The type of input this operator fetches
    type Input: Send + Sync;

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
/// Creates and configures the EVM EE and Checkpoint operators.
///
/// Returns: (CheckpointOperator, EvmEeOperator)
pub(crate) fn init_operators(
    evm_ee_client: HttpClient,
    cl_client: HttpClient,
) -> (CheckpointOperator, EvmEeOperator) {
    let evm_ee_operator = EvmEeOperator::new(evm_ee_client);
    let checkpoint_operator = CheckpointOperator::new(cl_client);

    (checkpoint_operator, evm_ee_operator)
}
