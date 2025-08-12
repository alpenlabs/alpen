//! Loads and formats OP block RPC response.

use reth_chainspec::{ChainSpec, ChainSpecProvider};
use reth_provider::HeaderProvider;
use reth_rpc_eth_api::{
    helpers::{EthBlocks, LoadBlock, LoadPendingBlock},
    FromEvmError, RpcConvert, RpcNodeCore,
};
use reth_rpc_eth_types::EthApiError;

use crate::{AlpenEthApi, StrataNodeCore};

impl<N, Rpc> EthBlocks for AlpenEthApi<N, Rpc>
where
    N: StrataNodeCore<Provider: ChainSpecProvider<ChainSpec = ChainSpec> + HeaderProvider>,
    EthApiError: FromEvmError<N::Evm>,
    Rpc: RpcConvert<Primitives = N::Primitives, Error = EthApiError>,
{
}

impl<N, Rpc> LoadBlock for AlpenEthApi<N, Rpc>
where
    Self: LoadPendingBlock,
    N: RpcNodeCore,
    Rpc: RpcConvert<Primitives = N::Primitives, Error = EthApiError>,
{
}
