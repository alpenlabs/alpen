use crate::chain_spec::{GChainSpec, Node, NodeHeader, NodeRef};

/// Provider that exposes the graph nodes for processing and topological
/// information about the graph for traversal.
pub trait ChainProvider {
    /// The chain spec this provider works for.
    type Spec: GChainSpec;

    /// Fetches the header for a node.
    fn fetch_node_header(
        &self,
        nref: &NodeRef<Self::Spec>,
    ) -> anyhow::Result<Option<NodeHeader<Self::Spec>>>;

    /// Fetches the full node, including header.
    fn fetch_node(
        &self,
        nref: &NodeRef<Self::Spec>,
    ) -> anyhow::Result<Option<NodeHeader<Self::Spec>>>;

    /// Fetches all the known node refs that are "forwards" in the graph from
    /// the specified node.
    ///
    /// MUST match the behavior of `fetch_backward_links`.
    fn fetch_forward_links(
        &self,
        nref: &NodeRef<Self::Spec>,
    ) -> anyhow::Result<Vec<NodeRef<Self::Spec>>>;

    /// Fetches all the known node refs that are "backwards" in the graph from
    /// the specified node.
    ///
    /// MUST match the behavior of `fetch_foward_links`.
    fn fetch_backward_links(
        &self,
        nref: &NodeRef<Self::Spec>,
    ) -> anyhow::Result<NodeRef<Self::Spec>>;
}
