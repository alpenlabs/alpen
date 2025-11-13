//! InputFetcher implementations for PaaS registry
//!
//! This module implements RegistryInputFetcher for each program type,
//! bridging between operators (which work with ProofContext) and PaaS
//! (which works with ProofTask).

use std::sync::Arc;

use strata_db_store_sled::prover::ProofDBSled;
use strata_paas::{PaaSError, PaaSResult, RegistryInputFetcher};
use strata_primitives::proof::ProofKey;
use strata_proofimpl_checkpoint::program::{CheckpointProgram, CheckpointProverInput};
use strata_proofimpl_cl_stf::program::{ClStfInput, ClStfProgram};
use strata_proofimpl_evm_ee_stf::{primitives::EvmEeProofInput, program::EvmEeProgram};

use crate::errors::ProvingTaskError;
use crate::operators::{checkpoint::CheckpointOperator, cl_stf::ClStfOperator, evm_ee::EvmEeOperator};

use super::task::ProofTask;
use super::get_current_backend;

/// Wrapper to add database dependency to CheckpointOperator
#[derive(Clone)]
pub(crate) struct CheckpointFetcher {
    pub(crate) operator: CheckpointOperator,
    pub(crate) db: Arc<ProofDBSled>,
}

impl RegistryInputFetcher<ProofTask, CheckpointProgram> for CheckpointFetcher {
    fn fetch_input<'a>(
        &'a self,
        program: &'a ProofTask,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = PaaSResult<CheckpointProverInput>> + Send + 'a>,
    > {
        Box::pin(async move {
            // Extract ProofContext from ProofTask wrapper
            let proof_context = program.0;
            let proof_key = ProofKey::new(proof_context, get_current_backend());
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

/// Wrapper to add database dependency to ClStfOperator
#[derive(Clone)]
pub(crate) struct ClStfFetcher {
    pub(crate) operator: ClStfOperator,
    pub(crate) db: Arc<ProofDBSled>,
}

impl RegistryInputFetcher<ProofTask, ClStfProgram> for ClStfFetcher {
    fn fetch_input<'a>(
        &'a self,
        program: &'a ProofTask,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = PaaSResult<ClStfInput>> + Send + 'a>>
    {
        Box::pin(async move {
            // Extract ProofContext from ProofTask wrapper
            let proof_context = program.0;
            let proof_key = ProofKey::new(proof_context, get_current_backend());
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

/// Wrapper to add database dependency to EvmEeOperator
#[derive(Clone)]
pub(crate) struct EvmEeFetcher {
    pub(crate) operator: EvmEeOperator,
    pub(crate) db: Arc<ProofDBSled>,
}

impl RegistryInputFetcher<ProofTask, EvmEeProgram> for EvmEeFetcher {
    fn fetch_input<'a>(
        &'a self,
        program: &'a ProofTask,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = PaaSResult<EvmEeProofInput>> + Send + 'a>,
    > {
        Box::pin(async move {
            // Extract ProofContext from ProofTask wrapper
            let proof_context = program.0;
            let proof_key = ProofKey::new(proof_context, get_current_backend());
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
