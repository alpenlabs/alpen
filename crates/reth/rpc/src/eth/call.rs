use alloy_rpc_types_eth::TransactionRequest;
use reth_evm::{block::BlockExecutorFactory, ConfigureEvm, EvmFactory, TxEnvFor};
use reth_node_api::NodePrimitives;
use reth_provider::{ProviderError, ProviderHeader, ProviderTx};
use reth_revm::context::TxEnv;
use reth_rpc_eth_api::{
    helpers::{estimate::EstimateCall, Call, EthCall, LoadBlock, LoadState, SpawnBlocking},
    FromEvmError, FullEthApiTypes, RpcConvert, RpcTypes,
};
use reth_rpc_eth_types::EthApiError;

use crate::{AlpenEthApi, StrataNodeCore};

impl<N> EthCall for AlpenEthApi<N>
where
    Self: EstimateCall + LoadBlock + FullEthApiTypes,
    N: StrataNodeCore,
{
}

impl<N> EstimateCall for AlpenEthApi<N>
where
    Self: Call,
    Self::Error: From<EthApiError>,
    N: StrataNodeCore,
{
}

impl<N> Call for AlpenEthApi<N>
where
    Self: LoadState<
            Evm: ConfigureEvm<
                Primitives: NodePrimitives<
                    BlockHeader = ProviderHeader<Self::Provider>,
                    SignedTx = ProviderTx<Self::Provider>,
                >,
                BlockExecutorFactory: BlockExecutorFactory<EvmFactory: EvmFactory<Tx = TxEnv>>,
            >,
            RpcConvert: RpcConvert<TxEnv = TxEnvFor<Self::Evm>, Network = Self::NetworkTypes>,
            NetworkTypes: RpcTypes<TransactionRequest: From<TransactionRequest>>,
            Error: FromEvmError<Self::Evm>
                       + From<<Self::RpcConvert as RpcConvert>::Error>
                       + From<ProviderError>,
        > + SpawnBlocking,
    Self::Error: From<EthApiError>,
    N: StrataNodeCore,
{
    #[inline]
    fn call_gas_limit(&self) -> u64 {
        self.inner.eth_api.gas_cap()
    }

    #[inline]
    fn max_simulate_blocks(&self) -> u64 {
        self.inner.eth_api.max_simulate_blocks()
    }
}
