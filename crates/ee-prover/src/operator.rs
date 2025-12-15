use std::sync::Arc;

use async_trait::async_trait;
use strata_paas::{InputFetcher, ProgramType};
use strata_primitives::proof::ProofContext;
use strata_proofimpl_eth_ee_acct::{
    prepare_proof_input, EthEeAcctDataProvider, EthEeAcctInput, UpdateId,
};

/// Input fetcher for EE account update proofs
///
/// This operator is responsible for fetching all required data for proof generation
/// using a data provider implementation.
#[derive(Clone)]
pub struct EthEeAcctOperator<D> {
    /// Data provider for fetching proof inputs
    data_provider: Arc<D>,
    /// Genesis configuration for the chain
    genesis: rsp_primitives::genesis::Genesis,
}

impl<D> EthEeAcctOperator<D>
where
    D: EthEeAcctDataProvider,
{
    /// Create a new operator with the given data provider and genesis configuration
    pub fn new(data_provider: Arc<D>, genesis: rsp_primitives::genesis::Genesis) -> Self {
        Self {
            data_provider,
            genesis,
        }
    }
}

#[async_trait]
impl<D, P> InputFetcher<P> for EthEeAcctOperator<D>
where
    D: EthEeAcctDataProvider + 'static,
    P: ProgramType + std::ops::Deref<Target = ProofContext>,
{
    type Input = EthEeAcctInput;
    type Error = strata_proofimpl_eth_ee_acct::DataProviderError;

    async fn fetch_input(&self, program: &P) -> Result<Self::Input, Self::Error> {
        let proof_context: &ProofContext = program;

        // Extract update_id from ProofContext
        let update_id: UpdateId = match proof_context {
            ProofContext::EthEeAcct(update_id) => *update_id,
            _ => {
                panic!(
                    "EthEeAcctOperator only handles EthEeAcct proofs, got: {:?}",
                    proof_context
                );
            }
        };

        // Fetch and assemble proof input
        // Note: This is a blocking operation but wrapped in async for the trait
        prepare_proof_input(self.data_provider.as_ref(), update_id, self.genesis.clone())
    }
}
