//! PaaS integration for prover-client using registry-based API
//!
//! This module implements the registry traits for ProofTask, bridging between
//! PaaS (which works with ProofTask) and the operators (which work with ProofContext).

use std::sync::Arc;

use strata_db_types::traits::ProofDatabase;
use strata_db_store_sled::prover::ProofDBSled;
use strata_paas::{PaaSError, PaaSResult, RegistryInputFetcher, RegistryProofStore};
use strata_primitives::proof::{ProofContext, ProofKey};
use strata_proofimpl_checkpoint::program::{CheckpointProgram, CheckpointProverInput};
use strata_proofimpl_cl_stf::program::{ClStfInput, ClStfProgram};
use strata_proofimpl_evm_ee_stf::{primitives::EvmEeProofInput, program::EvmEeProgram};
use zkaleido::ProofReceiptWithMetadata;

use crate::{
    errors::ProvingTaskError,
    operators::{checkpoint::CheckpointOperator, cl_stf::ClStfOperator, evm_ee::EvmEeOperator},
    proof_context_integration::ProofTask,
};

/// Convert ZkVmBackend to ProofZkVm
fn backend_to_zkvm(backend: strata_paas::ZkVmBackend) -> strata_primitives::proof::ProofZkVm {
    match backend {
        strata_paas::ZkVmBackend::SP1 => strata_primitives::proof::ProofZkVm::SP1,
        strata_paas::ZkVmBackend::Native => strata_primitives::proof::ProofZkVm::Native,
        strata_paas::ZkVmBackend::Risc0 => panic!("Risc0 not supported"),
    }
}

/// Get the current backend based on feature flags
fn get_current_backend() -> strata_primitives::proof::ProofZkVm {
    #[cfg(feature = "sp1")]
    {
        strata_primitives::proof::ProofZkVm::SP1
    }
    #[cfg(not(feature = "sp1"))]
    {
        strata_primitives::proof::ProofZkVm::Native
    }
}

// ===== InputFetcher Implementations for Operators =====

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

// ===== Unified Proof Store =====

/// Unified proof storage service that handles all proof types
///
/// This service:
/// - Stores proofs in the database
/// - Submits checkpoint proofs to the CL client
/// - Handles all proof types through the registry system
#[derive(Clone)]
pub(crate) struct ProofStoreService {
    db: Arc<ProofDBSled>,
    checkpoint_operator: CheckpointOperator,
}

impl ProofStoreService {
    pub(crate) fn new(db: Arc<ProofDBSled>, checkpoint_operator: CheckpointOperator) -> Self {
        Self {
            db,
            checkpoint_operator,
        }
    }
}

impl RegistryProofStore<ProofTask> for ProofStoreService {
    fn store_proof<'a>(
        &'a self,
        program: &'a ProofTask,
        proof: ProofReceiptWithMetadata,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = PaaSResult<()>> + Send + 'a>> {
        Box::pin(async move {
            // Extract ProofContext from ProofTask wrapper
            let proof_context = program.0;

            let backend = {
                #[cfg(feature = "sp1")]
                {
                    strata_paas::ZkVmBackend::SP1
                }
                #[cfg(not(feature = "sp1"))]
                {
                    strata_paas::ZkVmBackend::Native
                }
            };

            let proof_key = ProofKey::new(proof_context, backend_to_zkvm(backend));

            // Store proof in database
            self.db
                .put_proof(proof_key, proof)
                .map_err(|e| PaaSError::PermanentFailure(e.to_string()))?;

            // If this is a checkpoint proof, submit it to the CL client
            if let ProofContext::Checkpoint(checkpoint_idx) = proof_context {
                self.checkpoint_operator
                    .submit_checkpoint_proof(checkpoint_idx, &proof_key, &self.db)
                    .await
                    .map_err(|e| {
                        tracing::warn!(
                            %checkpoint_idx,
                            "Failed to submit checkpoint proof to CL: {}",
                            e
                        );
                        PaaSError::TransientFailure(format!(
                            "Checkpoint proof stored but CL submission failed: {}",
                            e
                        ))
                    })?;
            }

            Ok(())
        })
    }
}
