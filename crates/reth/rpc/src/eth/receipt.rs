//! Loads and formats Strata receipt RPC response.

use reth_rpc_eth_api::{helpers::LoadReceipt, RpcConvert, RpcNodeCore};
use reth_rpc_eth_types::EthApiError;

use crate::AlpenEthApi;

impl<N, Rpc> LoadReceipt for AlpenEthApi<N, Rpc>
where
    N: RpcNodeCore,
    Rpc: RpcConvert<Primitives = N::Primitives, Error = EthApiError>,
{
}
