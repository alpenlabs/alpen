use crate::chain_spec::*;
use crate::errors::ProviderError;

/// Provider that exposes the graph nodes for processing and topological
/// information about the graph for traversal.
///
/// A query with no answer returns `Ok(None)`; [`ProviderError`] means the
/// provider itself failed.
pub trait ChainProvider {
    /// The chain spec this provider works for.
    type Spec: GChainSpec;

    /// Fetches the header for a link.
    fn fetch_link_header(
        &self,
        lref: &LinkRef<Self::Spec>,
    ) -> Result<Option<LinkHeader<Self::Spec>>, ProviderError>;

    /// Fetches the full link, including header.
    ///
    /// The returned link MUST satisfy [`GLink::check_structurally_consistent`];
    /// a provider that finds otherwise reports
    /// [`ProviderError::InconsistentLink`] rather than handing corrupt data to
    /// the processor stages.
    fn fetch_link(
        &self,
        lref: &LinkRef<Self::Spec>,
    ) -> Result<Option<Link<Self::Spec>>, ProviderError>;

    /// Fetches the nodes a link connects.
    ///
    /// This is what makes the graph traversable: the target is the node whose
    /// forward links we look at next, and the origin is what tells us a link
    /// actually continues the path we're building.
    fn fetch_link_endpoints(
        &self,
        lref: &LinkRef<Self::Spec>,
    ) -> Result<Option<LinkEndpoints<Self::Spec>>, ProviderError>;

    /// Fetches all the known link refs that are "forwards" in the graph from
    /// the specified node.
    ///
    /// MUST match the behavior of `fetch_backward_links`.
    fn fetch_forward_links(
        &self,
        nref: &NodeRef<Self::Spec>,
    ) -> Result<Vec<LinkRef<Self::Spec>>, ProviderError>;

    /// Fetches all the known link refs that are "backwards" in the graph from
    /// the specified node.
    ///
    /// MUST match the behavior of `fetch_forward_links`.
    fn fetch_backward_links(
        &self,
        nref: &NodeRef<Self::Spec>,
    ) -> Result<Vec<LinkRef<Self::Spec>>, ProviderError>;
}
