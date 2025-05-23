use reth_chainspec::ChainSpec;
use reth_node_builder::{
    components::{BasicPayloadServiceBuilder, ComponentsBuilder},
    node::{FullNodeTypes, NodeTypes},
    rpc::RpcAddOns,
    Node, NodeAdapter, NodeComponentsBuilder,
};
use reth_node_ethereum::{
    node::{EthereumConsensusBuilder, EthereumNetworkBuilder, EthereumPoolBuilder},
    EthereumEthApiBuilder,
};
use reth_primitives::EthPrimitives;
use reth_provider::EthStorage;

use crate::{
    args::StrataNodeArgs, engine::StrataEngineValidatorBuilder, evm::StrataExecutorBuilder,
    payload_builder::StrataPayloadBuilderBuilder, StrataEngineTypes,
};

#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct StrataEthereumNode {
    // Strata node args.
    pub args: StrataNodeArgs,
}

impl StrataEthereumNode {
    /// Creates a new instance of the StrataEthereum node type.
    pub fn new(args: StrataNodeArgs) -> Self {
        Self { args }
    }
}

impl NodeTypes for StrataEthereumNode {
    type Primitives = EthPrimitives;
    type ChainSpec = ChainSpec;
    type StateCommitment = reth_trie_db::MerklePatriciaTrie;
    type Storage = EthStorage;
    type Payload = StrataEngineTypes;
}

impl<N> Node<N> for StrataEthereumNode
where
    N: FullNodeTypes<
        Types: NodeTypes<
            Payload = StrataEngineTypes,
            ChainSpec = ChainSpec,
            Primitives = EthPrimitives,
            Storage = EthStorage,
        >,
    >,
{
    type ComponentsBuilder = ComponentsBuilder<
        N,
        EthereumPoolBuilder,
        BasicPayloadServiceBuilder<StrataPayloadBuilderBuilder>,
        EthereumNetworkBuilder,
        StrataExecutorBuilder,
        EthereumConsensusBuilder,
    >;

    type AddOns = StrataNodeAddOns<
        NodeAdapter<N, <Self::ComponentsBuilder as NodeComponentsBuilder<N>>::Components>,
    >;

    fn components_builder(&self) -> Self::ComponentsBuilder {
        ComponentsBuilder::default()
            .node_types::<N>()
            .pool(EthereumPoolBuilder::default())
            .payload(BasicPayloadServiceBuilder::default())
            .network(EthereumNetworkBuilder::default())
            .executor(StrataExecutorBuilder::default())
            .consensus(EthereumConsensusBuilder::default())
    }

    fn add_ons(&self) -> Self::AddOns {
        Self::AddOns::default()
    }
}

/// Custom addons configuring RPC types
pub type StrataNodeAddOns<N> = RpcAddOns<N, EthereumEthApiBuilder, StrataEngineValidatorBuilder>;
