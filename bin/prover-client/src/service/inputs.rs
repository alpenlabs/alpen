//! Input providers for PaaS service
//!
//! This module implements InputProvider for each program type,
//! bridging between operators (which work with ProofContext) and PaaS
//! (which works with ProofTask).

use std::{future::Future, pin::Pin, sync::Arc};

use strata_db_store_sled::prover::ProofDBSled;
use strata_paas::{InputProvider, PaaSError, PaaSResult};
use strata_proofimpl_checkpoint::program::{CheckpointProgram, CheckpointProverInput};
use strata_proofimpl_cl_stf::program::{ClStfInput, ClStfProgram};
use strata_proofimpl_evm_ee_stf::{primitives::EvmEeProofInput, program::EvmEeProgram};

use crate::errors::ProvingTaskError;
use crate::operators::{checkpoint::CheckpointOperator, cl_stf::ClStfOperator, evm_ee::EvmEeOperator};

use super::proof_key_for;
use super::task::ProofTask;

/// Convert ProvingTaskError to PaaSError
///
/// Classifies errors as transient (retriable) or permanent based on the error type.
/// Transient errors include RPC failures and missing dependencies, which may resolve
/// on retry. All other errors are considered permanent.
fn to_paas_error(e: ProvingTaskError) -> PaaSError {
    match e {
        ProvingTaskError::RpcError(_)
        | ProvingTaskError::ProofNotFound(_)
        | ProvingTaskError::DependencyNotFound(_) => PaaSError::TransientFailure(e.to_string()),
        _ => PaaSError::PermanentFailure(e.to_string()),
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

// Implementation for CheckpointOperator
impl InputProvider<ProofTask, CheckpointProgram> for ProofInputProvider<CheckpointOperator> {
    fn provide_input<'a>(
        &'a self,
        program: &'a ProofTask,
    ) -> Pin<Box<dyn Future<Output = PaaSResult<CheckpointProverInput>> + Send + 'a>> {
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

// Implementation for ClStfOperator
impl InputProvider<ProofTask, ClStfProgram> for ProofInputProvider<ClStfOperator> {
    fn provide_input<'a>(
        &'a self,
        program: &'a ProofTask,
    ) -> Pin<Box<dyn Future<Output = PaaSResult<ClStfInput>> + Send + 'a>> {
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

// Implementation for EvmEeOperator
impl InputProvider<ProofTask, EvmEeProgram> for ProofInputProvider<EvmEeOperator> {
    fn provide_input<'a>(
        &'a self,
        program: &'a ProofTask,
    ) -> Pin<Box<dyn Future<Output = PaaSResult<EvmEeProofInput>> + Send + 'a>> {
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
