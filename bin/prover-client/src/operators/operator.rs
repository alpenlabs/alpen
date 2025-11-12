use std::sync::Arc;

use bitcoind_async_client::Client;
use jsonrpsee::http_client::HttpClient;
use strata_params::RollupParams;

use super::{checkpoint::CheckpointOperator, cl_stf::ClStfOperator, evm_ee::EvmEeOperator};

/// A struct that manages various proof operators, each corresponding to a distinct proof type.
///
/// The `ProofOperator` provides initialization and accessors for the underlying proof operators.
/// Actual proof generation is now handled by the PaaS (Prover-as-a-Service) framework.
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
