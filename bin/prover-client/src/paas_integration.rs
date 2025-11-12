//! PaaS integration for prover-client
//!
//! This module provides the integration between the prover-client and the PaaS library,
//! implementing the required traits for zkaleido-based proof generation.

use std::sync::Arc;

use strata_db_types::traits::ProofDatabase;
use strata_db_store_sled::prover::ProofDBSled;
use strata_paas::{InputFetcher, PaaSError, PaaSResult, ProofStore, ZkVmBackend};
use strata_primitives::proof::{ProofContext, ProofKey};
use strata_proofimpl_checkpoint::program::{CheckpointProgram, CheckpointProverInput};
use strata_proofimpl_cl_stf::program::{ClStfInput, ClStfProgram};
use strata_proofimpl_evm_ee_stf::{primitives::EvmEeProofInput, program::EvmEeProgram};
use strata_zkvm_hosts::ZkVmHostInstance;
use zkaleido::{ProofReceiptWithMetadata, ProofType, PublicValues, ZkVmHost, ZkVmInputResult, ZkVmProgram, ZkVmResult};

use crate::{
    errors::ProvingTaskError,
    operators::{checkpoint::CheckpointOperator, cl_stf::ClStfOperator, evm_ee::EvmEeOperator},
};

// Note: ProgramId implementation for ProofContext is in strata-paas/src/primitives.rs
// to avoid orphan rule violations.

/// Convert ProofZkVm to ZkVmBackend
pub(crate) fn zkvm_to_backend(vm: strata_primitives::proof::ProofZkVm) -> ZkVmBackend {
    match vm {
        strata_primitives::proof::ProofZkVm::SP1 => ZkVmBackend::SP1,
        strata_primitives::proof::ProofZkVm::Native => ZkVmBackend::Native,
        _ => panic!("Unsupported ZkVm backend: {:?}", vm),
    }
}

/// Convert ZkVmBackend to ProofZkVm
pub(crate) fn backend_to_zkvm(backend: ZkVmBackend) -> strata_primitives::proof::ProofZkVm {
    match backend {
        ZkVmBackend::SP1 => strata_primitives::proof::ProofZkVm::SP1,
        ZkVmBackend::Native => strata_primitives::proof::ProofZkVm::Native,
        ZkVmBackend::Risc0 => panic!("Risc0 not supported"),
    }
}

/// Enum that wraps all possible prover inputs
pub(crate) enum ProverInput {
    Checkpoint {
        input: CheckpointProverInput,
        proof_key: ProofKey,
        db: Arc<ProofDBSled>,
    },
    ClStf {
        input: ClStfInput,
        proof_key: ProofKey,
        db: Arc<ProofDBSled>,
    },
    EvmEeStf {
        input: EvmEeProofInput,
        proof_key: ProofKey,
        db: Arc<ProofDBSled>,
    },
}

/// Unified prover program that dispatches to the correct underlying program
pub(crate) struct ProverProgram;

impl ZkVmProgram for ProverProgram {
    type Input = ProverInput;
    type Output = Vec<u8>; // Generic output - actual output is stored in DB

    fn name() -> String {
        "unified_prover".to_string()
    }

    fn proof_type() -> ProofType {
        ProofType::Compressed
    }

    fn prepare_input<'a, B>(input: &'a Self::Input) -> ZkVmInputResult<B::Input>
    where
        B: zkaleido::ZkVmInputBuilder<'a>,
    {
        match input {
            ProverInput::Checkpoint { input, .. } => CheckpointProgram::prepare_input::<B>(input),
            ProverInput::ClStf { input, .. } => ClStfProgram::prepare_input::<B>(input),
            ProverInput::EvmEeStf { input, .. } => EvmEeProgram::prepare_input::<B>(input),
        }
    }

    fn process_output<H>(_public_values: &PublicValues) -> ZkVmResult<Self::Output>
    where
        H: ZkVmHost,
    {
        // We don't process output here - it's stored directly in the DB
        Ok(vec![])
    }

    fn prove<'a, H>(input: &'a Self::Input, host: &H) -> ZkVmResult<ProofReceiptWithMetadata>
    where
        H: ZkVmHost,
        H::Input<'a>: zkaleido::ZkVmInputBuilder<'a>,
    {
        // Dispatch to the appropriate program's prove method
        match input {
            ProverInput::Checkpoint { input, proof_key, db } => {
                let proof = CheckpointProgram::prove(input, host)?;
                db.put_proof(*proof_key, proof.clone())
                    .map_err(|e| {
                        zkaleido::ZkVmError::ProofGenerationError(format!(
                            "Failed to store proof: {}",
                            e
                        ))
                    })?;
                Ok(proof)
            }
            ProverInput::ClStf { input, proof_key, db } => {
                let proof = ClStfProgram::prove(input, host)?;
                db.put_proof(*proof_key, proof.clone())
                    .map_err(|e| {
                        zkaleido::ZkVmError::ProofGenerationError(format!(
                            "Failed to store proof: {}",
                            e
                        ))
                    })?;
                Ok(proof)
            }
            ProverInput::EvmEeStf { input, proof_key, db } => {
                let proof = EvmEeProgram::prove(input, host)?;
                db.put_proof(*proof_key, proof.clone())
                    .map_err(|e| {
                        zkaleido::ZkVmError::ProofGenerationError(format!(
                            "Failed to store proof: {}",
                            e
                        ))
                    })?;
                Ok(proof)
            }
        }
    }
}

/// Input fetcher that uses the existing operators
pub(crate) struct ProverInputFetcher {
    evm_ee_operator: Arc<EvmEeOperator>,
    cl_stf_operator: Arc<ClStfOperator>,
    checkpoint_operator: Arc<CheckpointOperator>,
    db: Arc<ProofDBSled>,
}

impl ProverInputFetcher {
    pub(crate) fn new(
        evm_ee_operator: EvmEeOperator,
        cl_stf_operator: ClStfOperator,
        checkpoint_operator: CheckpointOperator,
        db: Arc<ProofDBSled>,
    ) -> Self {
        Self {
            evm_ee_operator: Arc::new(evm_ee_operator),
            cl_stf_operator: Arc::new(cl_stf_operator),
            checkpoint_operator: Arc::new(checkpoint_operator),
            db,
        }
    }
}

impl InputFetcher<ProofContext> for ProverInputFetcher {
    type Program = ProverProgram;

    async fn fetch_input(
        &self,
        program: &ProofContext,
    ) -> PaaSResult<<Self::Program as ZkVmProgram>::Input> {
        // Determine which VM to use based on features
        let host = {
            #[cfg(feature = "sp1")]
            {
                strata_primitives::proof::ProofZkVm::SP1
            }
            #[cfg(not(feature = "sp1"))]
            {
                strata_primitives::proof::ProofZkVm::Native
            }
        };

        let proof_key = ProofKey::new(*program, host);

        match program {
            ProofContext::Checkpoint(..) => {
                let input = self
                    .checkpoint_operator
                    .fetch_input(&proof_key, &self.db)
                    .await
                    .map_err(|e| match e {
                        ProvingTaskError::RpcError(_)
                        | ProvingTaskError::ZkVmError(zkaleido::ZkVmError::NetworkRetryableError(_)) => {
                            PaaSError::TransientFailure(e.to_string())
                        }
                        _ => PaaSError::PermanentFailure(e.to_string()),
                    })?;

                Ok(ProverInput::Checkpoint {
                    input,
                    proof_key,
                    db: self.db.clone(),
                })
            }
            ProofContext::ClStf(..) => {
                let input = self
                    .cl_stf_operator
                    .fetch_input(&proof_key, &self.db)
                    .await
                    .map_err(|e| match e {
                        ProvingTaskError::RpcError(_)
                        | ProvingTaskError::ZkVmError(zkaleido::ZkVmError::NetworkRetryableError(_)) => {
                            PaaSError::TransientFailure(e.to_string())
                        }
                        _ => PaaSError::PermanentFailure(e.to_string()),
                    })?;

                Ok(ProverInput::ClStf {
                    input,
                    proof_key,
                    db: self.db.clone(),
                })
            }
            ProofContext::EvmEeStf(..) => {
                let input = self
                    .evm_ee_operator
                    .fetch_input(&proof_key, &self.db)
                    .await
                    .map_err(|e| match e {
                        ProvingTaskError::RpcError(_)
                        | ProvingTaskError::ZkVmError(zkaleido::ZkVmError::NetworkRetryableError(_)) => {
                            PaaSError::TransientFailure(e.to_string())
                        }
                        _ => PaaSError::PermanentFailure(e.to_string()),
                    })?;

                Ok(ProverInput::EvmEeStf {
                    input,
                    proof_key,
                    db: self.db.clone(),
                })
            }
        }
    }
}

/// Proof store that uses ProofDBSled
pub(crate) struct ProverProofStore {
    db: Arc<ProofDBSled>,
}

impl ProverProofStore {
    pub(crate) fn new(db: Arc<ProofDBSled>) -> Self {
        Self { db }
    }
}

impl ProofStore<ProofContext> for ProverProofStore {
    async fn store_proof(
        &self,
        task_id: &strata_paas::ZkVmTaskId<ProofContext>,
        proof: ProofReceiptWithMetadata,
    ) -> PaaSResult<()> {
        let proof_key = ProofKey::new(task_id.program, backend_to_zkvm(task_id.backend.clone()));

        self.db
            .put_proof(proof_key, proof)
            .map_err(|e| PaaSError::PermanentFailure(e.to_string()))?;

        Ok(())
    }
}

/// Custom Prover implementation that resolves hosts dynamically per task
///
/// This is necessary because we need to support multiple ZkVM backends (Native, SP1)
/// and resolve the appropriate host based on the task's backend.
pub(crate) struct DynamicHostProver {
    input_fetcher: Arc<ProverInputFetcher>,
    proof_store: Arc<ProverProofStore>,
}

impl DynamicHostProver {
    pub(crate) fn new(
        input_fetcher: Arc<ProverInputFetcher>,
        proof_store: Arc<ProverProofStore>,
    ) -> Self {
        Self {
            input_fetcher,
            proof_store,
        }
    }

    /// Resolve the host for a given task
    fn resolve_host(backend: &strata_paas::ZkVmBackend) -> ZkVmHostInstance {
        // Create a dummy proof key to resolve the host
        // We just need any proof context since the host is determined by the backend
        let zkvm = backend_to_zkvm(backend.clone());
        let proof_key = ProofKey::new(ProofContext::Checkpoint(0), zkvm);
        strata_zkvm_hosts::resolve_host(&proof_key)
    }

    /// Prove using the resolved host
    async fn prove_with_host(
        &self,
        task_id: strata_paas::ZkVmTaskId<ProofContext>,
    ) -> PaaSResult<()> {
        // Fetch input
        let input = self
            .input_fetcher
            .fetch_input(&task_id.program)
            .await?;

        // Resolve host based on backend
        let host = Self::resolve_host(&task_id.backend);

        // Prove using the host
        let proof = match host {
            ZkVmHostInstance::Native(ref h) => ProverProgram::prove(&input, h),
            #[cfg(feature = "sp1")]
            ZkVmHostInstance::SP1(h) => ProverProgram::prove(&input, h),
            #[cfg(feature = "sp1")]
            _ => panic!("Unsupported host variant"),
            #[cfg(not(feature = "sp1"))]
            _ => panic!("Unsupported host variant"),
        }
        .map_err(|e| PaaSError::PermanentFailure(format!("Proving failed: {}", e)))?;

        // Store the proof
        self.proof_store.store_proof(&task_id, proof).await?;

        Ok(())
    }
}

impl strata_paas::Prover for DynamicHostProver {
    type TaskId = strata_paas::ZkVmTaskId<ProofContext>;
    type Backend = strata_paas::ZkVmBackend;

    fn backend(&self, task_id: &Self::TaskId) -> Self::Backend {
        task_id.backend.clone()
    }

    async fn prove(&self, task_id: Self::TaskId) -> PaaSResult<()> {
        self.prove_with_host(task_id).await
    }
}
