//! Strata `eth_` endpoint implementation.
//! adapted from reth-node-optimism::rpc

pub mod receipt;
pub mod transaction;

mod block;
mod call;
mod pending_block;

use std::{fmt, sync::Arc};

use alloy_primitives::U256;
use reth_chainspec::{EthChainSpec, EthereumHardforks};
use reth_evm::ConfigureEvm;
use reth_network_api::NetworkInfo;
use reth_node_api::{FullNodeComponents, NodePrimitives};
use reth_node_builder::rpc::{EthApiBuilder, EthApiCtx};
use reth_primitives::EthPrimitives;
use reth_provider::{
    BlockNumReader, BlockReader, BlockReaderIdExt, CanonStateSubscriptions, ChainSpecProvider,
    ProviderBlock, ProviderHeader, ProviderReceipt, ProviderTx, StageCheckpointReader,
    StateProviderFactory,
};
use reth_rpc::eth::{core::EthApiInner, DevSigner};
use reth_rpc_eth_api::{
    helpers::{
        AddDevSigners, EthApiSpec, EthFees, EthSigner, EthState, LoadBlock, LoadFee, LoadState,
        SpawnBlocking, Trace,
    },
    EthApiTypes, FromEvmError, FullEthApiServer, RpcNodeCore, RpcNodeCoreExt,
};
use reth_rpc_eth_types::{EthApiError, EthStateCache, FeeHistoryCache, GasPriceOracle};
use reth_tasks::{
    pool::{BlockingTaskGuard, BlockingTaskPool},
    TaskSpawner,
};
use reth_transaction_pool::TransactionPool;

use crate::SequencerClient;

/// Adapter for [`EthApiInner`], which holds all the data required to serve core `eth_` API.
pub type EthApiNodeBackend<N> = EthApiInner<
    <N as RpcNodeCore>::Provider,
    <N as RpcNodeCore>::Pool,
    <N as RpcNodeCore>::Network,
    <N as RpcNodeCore>::Evm,
>;

/// A helper trait with requirements for [`RpcNodeCore`] to be used in [`StrataEthApi`].
pub trait StrataNodeCore: RpcNodeCore<Provider: BlockReader> {}
impl<T> StrataNodeCore for T where T: RpcNodeCore<Provider: BlockReader> {}

/// Strata Eth API implementation.
///
/// This type provides the functionality for handling `eth_` related requests.
///
/// This wraps a default `Eth` implementation, and provides additional functionality where the
/// Strata spec deviates from the default (ethereum) spec, e.g. transaction forwarding to the
/// sequencer.
///
/// This type implements the [`FullEthApi`](reth_rpc_eth_api::helpers::FullEthApi) by implemented
/// all the `Eth` helper traits and prerequisite traits.
#[derive(Clone)]
pub struct StrataEthApi<N: StrataNodeCore> {
    /// Gateway to node's core components.
    inner: Arc<StrataEthApiInner<N>>,
}

impl<N> StrataEthApi<N>
where
    N: StrataNodeCore<
        Provider: BlockReaderIdExt
                      + ChainSpecProvider
                      + CanonStateSubscriptions<Primitives = EthPrimitives>
                      + Clone
                      + 'static,
    >,
{
    /// Returns a reference to the [`EthApiNodeBackend`].
    pub fn eth_api(&self) -> &EthApiNodeBackend<N> {
        self.inner.eth_api()
    }

    /// Returns the configured sequencer client, if any.
    pub fn sequencer_client(&self) -> Option<&SequencerClient> {
        self.inner.sequencer_client()
    }

    /// Build a [`StrataEthApi`] using [`StrataEthApiBuilder`].
    pub fn builder() -> StrataEthApiBuilder {
        StrataEthApiBuilder::new()
    }
}

impl<N> EthApiTypes for StrataEthApi<N>
where
    Self: Send + Sync,
    N: StrataNodeCore,
{
    type Error = EthApiError;
    type NetworkTypes = alloy_network::Ethereum;
    type TransactionCompat = Self;

    fn tx_resp_builder(&self) -> &Self::TransactionCompat {
        self
    }
}

impl<N> RpcNodeCore for StrataEthApi<N>
where
    N: StrataNodeCore,
{
    type Primitives = EthPrimitives;
    type Provider = N::Provider;
    type Pool = N::Pool;
    type Evm = <N as RpcNodeCore>::Evm;
    type Network = <N as RpcNodeCore>::Network;
    type PayloadBuilder = ();

    #[inline]
    fn pool(&self) -> &Self::Pool {
        self.inner.eth_api.pool()
    }

    #[inline]
    fn evm_config(&self) -> &Self::Evm {
        self.inner.eth_api.evm_config()
    }

    #[inline]
    fn network(&self) -> &Self::Network {
        self.inner.eth_api.network()
    }

    #[inline]
    fn payload_builder(&self) -> &Self::PayloadBuilder {
        &()
    }

    #[inline]
    fn provider(&self) -> &Self::Provider {
        self.inner.eth_api.provider()
    }
}

impl<N> RpcNodeCoreExt for StrataEthApi<N>
where
    N: StrataNodeCore,
{
    #[inline]
    fn cache(&self) -> &EthStateCache<ProviderBlock<N::Provider>, ProviderReceipt<N::Provider>> {
        self.inner.eth_api.cache()
    }
}

impl<N> EthApiSpec for StrataEthApi<N>
where
    N: StrataNodeCore<
        Provider: ChainSpecProvider<ChainSpec: EthereumHardforks>
                      + BlockNumReader
                      + StageCheckpointReader,
        Network: NetworkInfo,
    >,
{
    type Transaction = ProviderTx<Self::Provider>;

    #[inline]
    fn starting_block(&self) -> U256 {
        self.inner.eth_api.starting_block()
    }

    #[inline]
    fn signers(&self) -> &parking_lot::RwLock<Vec<Box<dyn EthSigner<ProviderTx<Self::Provider>>>>> {
        self.inner.eth_api.signers()
    }
}

impl<N> SpawnBlocking for StrataEthApi<N>
where
    Self: Send + Sync + Clone + 'static,
    N: StrataNodeCore,
{
    #[inline]
    fn io_task_spawner(&self) -> impl TaskSpawner {
        self.inner.eth_api.task_spawner()
    }

    #[inline]
    fn tracing_task_pool(&self) -> &BlockingTaskPool {
        self.inner.eth_api.blocking_task_pool()
    }

    #[inline]
    fn tracing_task_guard(&self) -> &BlockingTaskGuard {
        self.inner.eth_api.blocking_task_guard()
    }
}

impl<N> LoadFee for StrataEthApi<N>
where
    Self: LoadBlock<Provider = N::Provider>,
    N: StrataNodeCore<
        Provider: BlockReaderIdExt
                      + ChainSpecProvider<ChainSpec: EthChainSpec + EthereumHardforks>
                      + StateProviderFactory,
    >,
{
    #[inline]
    fn gas_oracle(&self) -> &GasPriceOracle<Self::Provider> {
        self.inner.eth_api.gas_oracle()
    }

    #[inline]
    fn fee_history_cache(&self) -> &FeeHistoryCache {
        self.inner.eth_api.fee_history_cache()
    }
}

impl<N> LoadState for StrataEthApi<N> where
    N: StrataNodeCore<
        Provider: StateProviderFactory + ChainSpecProvider<ChainSpec: EthereumHardforks>,
        Pool: TransactionPool,
    >
{
}

impl<N> EthState for StrataEthApi<N>
where
    Self: LoadState + SpawnBlocking,
    N: StrataNodeCore,
{
    #[inline]
    fn max_proof_window(&self) -> u64 {
        self.inner.eth_api.eth_proof_window()
    }
}

impl<N> EthFees for StrataEthApi<N>
where
    Self: LoadFee,
    N: StrataNodeCore,
{
}

impl<N> Trace for StrataEthApi<N>
where
    Self: RpcNodeCore<Provider: BlockReader>
        + LoadState<
            Evm: ConfigureEvm<
                Primitives: NodePrimitives<
                    BlockHeader = ProviderHeader<Self::Provider>,
                    SignedTx = ProviderTx<Self::Provider>,
                >,
            >,
            Error: FromEvmError<Self::Evm>,
        >,
    N: StrataNodeCore,
{
}

impl<N> AddDevSigners for StrataEthApi<N>
where
    N: StrataNodeCore,
{
    fn with_dev_accounts(&self) {
        *self.inner.eth_api.signers().write() = DevSigner::random_signers(20)
    }
}

impl<N: StrataNodeCore> fmt::Debug for StrataEthApi<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StrataEthApi").finish_non_exhaustive()
    }
}

/// Container type for [`StrataEthApi`]
#[allow(missing_debug_implementations)]
struct StrataEthApiInner<N: StrataNodeCore> {
    /// Gateway to node's core components.
    eth_api: EthApiNodeBackend<N>,
    /// Sequencer client, configured to forward submitted transactions to sequencer of given OP
    /// network.
    sequencer_client: Option<SequencerClient>,
}

impl<N: StrataNodeCore> StrataEthApiInner<N> {
    /// Returns a reference to the [`EthApiNodeBackend`].
    const fn eth_api(&self) -> &EthApiNodeBackend<N> {
        &self.eth_api
    }

    /// Returns the configured sequencer client, if any.
    const fn sequencer_client(&self) -> Option<&SequencerClient> {
        self.sequencer_client.as_ref()
    }
}

#[derive(Default)]
pub struct StrataEthApiBuilder {
    /// Sequencer client, configured to forward submitted transactions to sequencer of given OP
    /// network.
    sequencer_client: Option<SequencerClient>,
}

impl StrataEthApiBuilder {
    /// Creates a [`StrataEthApiBuilder`] instance.
    pub fn new() -> Self {
        Self {
            sequencer_client: None,
        }
    }

    /// With a [`SequencerClient`].
    pub fn with_sequencer(mut self, sequencer_client: Option<SequencerClient>) -> Self {
        self.sequencer_client = sequencer_client;
        self
    }
}

impl<N> EthApiBuilder<N> for StrataEthApiBuilder
where
    N: FullNodeComponents,
    StrataEthApi<N>: FullEthApiServer<Provider = N::Provider, Pool = N::Pool>,
{
    type EthApi = StrataEthApi<N>;

    async fn build_eth_api(self, ctx: EthApiCtx<'_, N>) -> eyre::Result<Self::EthApi> {
        let Self { sequencer_client } = self;
        let eth_api = reth_rpc::EthApiBuilder::new(
            ctx.components.provider().clone(),
            ctx.components.pool().clone(),
            ctx.components.network().clone(),
            ctx.components.evm_config().clone(),
        )
        .eth_cache(ctx.cache)
        .task_spawner(ctx.components.task_executor().clone())
        .gas_cap(ctx.config.rpc_gas_cap.into())
        .max_simulate_blocks(ctx.config.rpc_max_simulate_blocks)
        .eth_proof_window(ctx.config.eth_proof_window)
        .fee_history_cache_config(ctx.config.fee_history_cache)
        .proof_permits(ctx.config.proof_permits)
        .gas_oracle_config(ctx.config.gas_oracle)
        .build_inner();

        Ok(StrataEthApi {
            inner: Arc::new(StrataEthApiInner {
                eth_api,
                sequencer_client,
            }),
        })
    }
}
