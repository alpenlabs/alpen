//! Input providers for PaaS service
//!
//! This module implements InputProvider for each program type,
//! bridging between operators (which work with ProofContext) and PaaS
//! (which works with ProofTask).

use std::{future::Future, pin::Pin, sync::Arc};

use strata_db_store_sled::prover::ProofDBSled;
use strata_paas::{InputProvider, PaaSError, PaaSResult};
use strata_primitives::proof::ProofKey;
use strata_proofimpl_checkpoint::program::{CheckpointProgram, CheckpointProverInput};
use strata_proofimpl_cl_stf::program::{ClStfInput, ClStfProgram};
use strata_proofimpl_evm_ee_stf::{primitives::EvmEeProofInput, program::EvmEeProgram};

use crate::errors::ProvingTaskError;
use crate::operators::{checkpoint::CheckpointOperator, cl_stf::ClStfOperator, evm_ee::EvmEeOperator};

use super::current_zkvm;
use super::task::ProofTask;

/// Input provider for checkpoint proofs
#[derive(Clone)]
pub(crate) struct CheckpointInputProvider {
    pub(crate) operator: CheckpointOperator,
    pub(crate) db: Arc<ProofDBSled>,
}

impl InputProvider<ProofTask, CheckpointProgram> for CheckpointInputProvider {
    fn provide_input<'a>(
        &'a self,
        program: &'a ProofTask,
    ) -> Pin<Box<dyn Future<Output = PaaSResult<CheckpointProverInput>> + Send + 'a>> {
        Box::pin(async move {
            // Extract ProofContext from ProofTask wrapper
            let proof_context = program.0;
            let proof_key = ProofKey::new(proof_context, current_zkvm());
            self.operator
                .fetch_input(&proof_key, &self.db)
                .await
                .map_err(|e| match e {
                    ProvingTaskError::RpcError(_)
                    | ProvingTaskError::ProofNotFound(_)
                    | ProvingTaskError::DependencyNotFound(_) => {
                        PaaSError::TransientFailure(e.to_string())
                    }
                    _ => PaaSError::PermanentFailure(e.to_string()),
                })
        })
    }
}

/// Input provider for CL STF proofs
#[derive(Clone)]
pub(crate) struct ClStfInputProvider {
    pub(crate) operator: ClStfOperator,
    pub(crate) db: Arc<ProofDBSled>,
}

impl InputProvider<ProofTask, ClStfProgram> for ClStfInputProvider {
    fn provide_input<'a>(
        &'a self,
        program: &'a ProofTask,
    ) -> Pin<Box<dyn Future<Output = PaaSResult<ClStfInput>> + Send + 'a>> {
        Box::pin(async move {
            // Extract ProofContext from ProofTask wrapper
            let proof_context = program.0;
            let proof_key = ProofKey::new(proof_context, current_zkvm());
            self.operator
                .fetch_input(&proof_key, &self.db)
                .await
                .map_err(|e| match e {
                    ProvingTaskError::RpcError(_)
                    | ProvingTaskError::ProofNotFound(_)
                    | ProvingTaskError::DependencyNotFound(_) => {
                        PaaSError::TransientFailure(e.to_string())
                    }
                    _ => PaaSError::PermanentFailure(e.to_string()),
                })
        })
    }
}

/// Input provider for EVM EE proofs
#[derive(Clone)]
pub(crate) struct EvmEeInputProvider {
    pub(crate) operator: EvmEeOperator,
    pub(crate) db: Arc<ProofDBSled>,
}

impl InputProvider<ProofTask, EvmEeProgram> for EvmEeInputProvider {
    fn provide_input<'a>(
        &'a self,
        program: &'a ProofTask,
    ) -> Pin<Box<dyn Future<Output = PaaSResult<EvmEeProofInput>> + Send + 'a>> {
        Box::pin(async move {
            // Extract ProofContext from ProofTask wrapper
            let proof_context = program.0;
            let proof_key = ProofKey::new(proof_context, current_zkvm());
            self.operator
                .fetch_input(&proof_key, &self.db)
                .await
                .map_err(|e| match e {
                    ProvingTaskError::RpcError(_)
                    | ProvingTaskError::ProofNotFound(_)
                    | ProvingTaskError::DependencyNotFound(_) => {
                        PaaSError::TransientFailure(e.to_string())
                    }
                    _ => PaaSError::PermanentFailure(e.to_string()),
                })
        })
    }
}
