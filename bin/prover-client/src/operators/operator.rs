use std::sync::Arc;

use bitcoind_async_client::Client;
use jsonrpsee::http_client::HttpClient;
use strata_db_store_sled::prover::ProofDBSled;
use strata_primitives::{params::RollupParams, proof::ProofContext};
use strata_rpc_types::ProofKey;
use strata_zkvm_hosts::{resolve_host, ZkVmHostInstance};

use super::{
    checkpoint::CheckpointOperator, cl_stf::ClStfOperator, evm_ee::EvmEeOperator, ProvingOp,
};
use crate::errors::ProvingTaskError;

/// A struct that manages various proof operators, each corresponding to a distinct proof type.
///
/// The `ProofOperator` provides initialization, accessors, and methods for orchestrating
/// proof generation and processing. It is designed to encapsulate multiple operators
/// implementing the `ProvingOp` trait while handling the trait's lack of object safety
/// by organizing operations through this struct.
#[derive(Debug, Clone)]
pub(crate) struct ProofOperator {
    evm_ee_operator: EvmEeOperator,
    cl_stf_operator: ClStfOperator,
    checkpoint_operator: CheckpointOperator,
}

impl ProofOperator {
    /// Creates a new instance of `ProofOperator` with the provided proof operators.
    pub(crate) fn new(
        evm_ee_operator: EvmEeOperator,
        cl_stf_operator: ClStfOperator,
        checkpoint_operator: CheckpointOperator,
    ) -> Self {
        Self {
            evm_ee_operator,
            cl_stf_operator,
            checkpoint_operator,
        }
    }

    /// Initializes a `ProofOperator` by creating and configuring the underlying proof operators.
    pub(crate) fn init(
        btc_client: Client,
        evm_ee_client: HttpClient,
        cl_client: HttpClient,
        rollup_params: RollupParams,
        enable_checkpoint_runner: bool,
    ) -> Self {
        let _btc_client = Arc::new(btc_client);
        let rollup_params = Arc::new(rollup_params);

        let evm_ee_operator = EvmEeOperator::new(evm_ee_client.clone());
        let cl_stf_operator = ClStfOperator::new(
            cl_client.clone(),
            Arc::new(evm_ee_operator.clone()),
            rollup_params.clone(),
        );
        let checkpoint_operator = CheckpointOperator::new(
            cl_client.clone(),
            Arc::new(cl_stf_operator.clone()),
            enable_checkpoint_runner,
        );

        ProofOperator::new(evm_ee_operator, cl_stf_operator, checkpoint_operator)
    }

    /// Asynchronously generates a proof using the specified operator and host environment.
    pub(crate) async fn prove(
        operator: &impl ProvingOp,
        proof_key: &ProofKey,
        db: &ProofDBSled,
        host: ZkVmHostInstance,
    ) -> Result<(), ProvingTaskError> {
        match host {
            ZkVmHostInstance::Native(host) => operator.prove(proof_key, db, &host).await,

            #[cfg(feature = "sp1")]
            ZkVmHostInstance::SP1(host) => operator.prove(proof_key, db, host).await,

            #[cfg(feature = "risc0")]
            ZkVmHostInstance::Risc0(host) => operator.prove(proof_key, db, host).await,

            _ => unreachable!("Unexpected host variant"),
        }
    }

    /// Processes a proof generation task by delegating to the appropriate proof operator.
    pub(crate) async fn process_proof(
        &self,
        proof_key: &ProofKey,
        db: &ProofDBSled,
    ) -> Result<(), ProvingTaskError> {
        let host = resolve_host(proof_key);

        match proof_key.context() {
            ProofContext::EvmEeStf(..) => {
                Self::prove(&self.evm_ee_operator, proof_key, db, host).await
            }
            ProofContext::ClStf(..) => {
                Self::prove(&self.cl_stf_operator, proof_key, db, host).await
            }
            ProofContext::Checkpoint(..) => {
                Self::prove(&self.checkpoint_operator, proof_key, db, host).await
            }
        }
    }

    /// Returns a reference to the [`EvmEeOperator`].
    pub(crate) fn evm_ee_operator(&self) -> &EvmEeOperator {
        &self.evm_ee_operator
    }

    /// Returns a reference to the [`ClStfOperator`].
    pub(crate) fn cl_stf_operator(&self) -> &ClStfOperator {
        &self.cl_stf_operator
    }

    /// Returns a reference to the [`CheckpointOperator`].
    pub(crate) fn checkpoint_operator(&self) -> &CheckpointOperator {
        &self.checkpoint_operator
    }
}
