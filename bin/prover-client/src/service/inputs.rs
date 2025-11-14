//! Input providers for Prover Service
//!
//! This module implements InputProvider for each program type,
//! bridging between operators (which work with ProofContext) and PaaS
//! (which works with ProofTask).

use std::{future::Future, pin::Pin, sync::Arc};

use strata_db_store_sled::prover::ProofDBSled;
use strata_paas::{InputProvider, ProverServiceError, ProverServiceResult};
use zkaleido::ZkVmProgram;

use super::{proof_key_for, task::ProofTask};
use crate::{
    errors::ProvingTaskError,
    operators::{
        checkpoint::CheckpointOperator, cl_stf::ClStfOperator, evm_ee::EvmEeOperator,
        ProofInputFetcher,
    },
};

/// Convert ProvingTaskError to ProverServiceError
///
/// Classifies errors as transient (retriable) or permanent based on the error type.
/// Transient errors include RPC failures and missing dependencies, which may resolve
/// on retry. All other errors are considered permanent.
fn to_paas_error(e: ProvingTaskError) -> ProverServiceError {
    match e {
        ProvingTaskError::RpcError(_)
        | ProvingTaskError::ProofNotFound(_)
        | ProvingTaskError::DependencyNotFound(_) => {
            ProverServiceError::TransientFailure(e.to_string())
        }
        _ => ProverServiceError::PermanentFailure(e.to_string()),
    }
}

/// Generic input provider for proof operators
///
/// This provider works with any operator implementing [`ProofInputFetcher`],
/// eliminating code duplication across different proof types.
#[derive(Clone)]
pub(crate) struct ProofInputProvider<O> {
    pub(crate) operator: O,
    pub(crate) db: Arc<ProofDBSled>,
}

impl<O> ProofInputProvider<O> {
    /// Create a new input provider with the given operator and database
    pub(crate) fn new(operator: O, db: Arc<ProofDBSled>) -> Self {
        Self { operator, db }
    }
}

// Generic implementation for any operator implementing ProofInputFetcher
impl<O, Prog> InputProvider<ProofTask, Prog> for ProofInputProvider<O>
where
    O: ProofInputFetcher + Clone + 'static,
    Prog: ZkVmProgram<Input = O::Input>,
{
    fn provide_input<'a>(
        &'a self,
        program: &'a ProofTask,
    ) -> Pin<Box<dyn Future<Output = ProverServiceResult<Prog::Input>> + Send + 'a>> {
        Box::pin(async move {
            let proof_context = program.0;
            let proof_key = proof_key_for(proof_context);
            self.operator
                .fetch_input(&proof_key, &self.db)
                .await
                .map_err(to_paas_error)
        })
    }
}

// Type aliases for backward compatibility and clarity
pub(crate) type CheckpointInputProvider = ProofInputProvider<CheckpointOperator>;
pub(crate) type ClStfInputProvider = ProofInputProvider<ClStfOperator>;
pub(crate) type EvmEeInputProvider = ProofInputProvider<EvmEeOperator>;
