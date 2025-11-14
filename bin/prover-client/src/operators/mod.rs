//! A module defining operations for proof generation using ZKVMs.
//!
//! This module provides operators that encapsulate RPC client accessors
//! for fetching data needed for proof generation.
//!
//! NOTE: The original ProvingOp trait and task creation methods have been removed
//! as they are now handled by the PaaS (Prover-as-a-Service) framework.
//! This module now only contains minimal accessor methods for RPC clients.
//!
//! Supported ZKVMs:
//!
//! - Native
//! - SP1 (requires `sp1` feature enabled)

use strata_db_store_sled::prover::ProofDBSled;
use strata_primitives::proof::ProofKey;

use crate::errors::ProvingTaskError;

pub(crate) mod checkpoint;
pub(crate) mod cl_stf;
pub(crate) mod evm_ee;
pub(crate) mod operator;

pub(crate) use operator::ProofOperator;

/// Trait for operators that can fetch proof inputs
///
/// This provides a unified interface for all proof operators to fetch
/// the inputs required for proof generation. All operators (Checkpoint,
/// ClStf, EvmEe) implement this trait, establishing a common contract.
#[allow(dead_code)]
pub(crate) trait ProofInputFetcher {
    /// The type of input this operator fetches
    type Input;

    /// Fetch the input required for proof generation
    ///
    /// # Arguments
    ///
    /// * `task_id` - The proof key identifying what to prove
    /// * `db` - The proof database for retrieving dependencies
    async fn fetch_input(
        &self,
        task_id: &ProofKey,
        db: &ProofDBSled,
    ) -> Result<Self::Input, ProvingTaskError>;
}

