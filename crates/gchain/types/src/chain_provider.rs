use crate::chain_spec::*;

/// Provider that exposes the graph nodes for processing and topological
/// information about the graph for traversal.
pub trait ChainProvider {
    /// The chain spec this provider works for.
    type Spec: GChainSpec;

    /// Fetches the header for a link.
    fn fetch_link_header(
        &self,
        lref: &LinkRef<Self::Spec>,
    ) -> anyhow::Result<Option<LinkHeader<Self::Spec>>>;

    /// Fetches the full link, including header.
    fn fetch_link(&self, lref: &LinkRef<Self::Spec>) -> anyhow::Result<Option<Link<Self::Spec>>>;

    /// Fetches all the known link refs that are "forwards" in the graph from
    /// the specified node.
    ///
    /// MUST match the behavior of `fetch_backward_links`.
    fn fetch_forward_links(
        &self,
        nref: &NodeRef<Self::Spec>,
    ) -> anyhow::Result<Vec<LinkRef<Self::Spec>>>;

    /// Fetches all the known link refs that are "backwards" in the graph from
    /// the specified node.
    ///
    /// MUST match the behavior of `fetch_foward_links`.
    fn fetch_backward_links(
        &self,
        nref: &NodeRef<Self::Spec>,
    ) -> anyhow::Result<LinkRef<Self::Spec>>;
}
