use std::fmt::{self, Debug};

use crate::chain_provider::ChainProvider;
use crate::chain_spec::*;
use crate::errors::ProviderError;

/// The nodes a link connects.
///
/// A link ref on its own only names a transition, it doesn't say where in the
/// graph that transition sits.  These are what let a sync driver walk the graph:
/// the target of the link it just traversed is the node it fetches the next
/// forward links from.  They're also what makes it possible to notice that a
/// checkpoint step and a path of block steps converge on the same node.
///
/// Not every chain can derive these from a link header alone, so they're fetched
/// from the provider (see
/// [`ChainProvider::fetch_link_endpoints`](crate::ChainProvider::fetch_link_endpoints)).
pub struct LinkEndpoints<S: GChainSpec> {
    origin: NodeRef<S>,
    target: NodeRef<S>,
}

impl<S: GChainSpec> LinkEndpoints<S> {
    pub fn new(origin: NodeRef<S>, target: NodeRef<S>) -> Self {
        Self { origin, target }
    }

    /// The node the link departs from.
    pub fn origin(&self) -> &NodeRef<S> {
        &self.origin
    }

    /// The node the link arrives at.
    pub fn target(&self) -> &NodeRef<S> {
        &self.target
    }
}

// Implemented by hand so that the bounds fall on the ref types rather than on
// the spec type, which is only ever a marker.
impl<S: GChainSpec> Clone for LinkEndpoints<S> {
    fn clone(&self) -> Self {
        Self::new(self.origin.clone(), self.target.clone())
    }
}

impl<S: GChainSpec> Debug for LinkEndpoints<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LinkEndpoints")
            .field("origin", &self.origin)
            .field("target", &self.target)
            .finish()
    }
}

impl<S: GChainSpec> PartialEq for LinkEndpoints<S> {
    fn eq(&self, other: &Self) -> bool {
        self.origin == other.origin && self.target == other.target
    }
}

impl<S: GChainSpec> Eq for LinkEndpoints<S> {}

/// Describes a path through the node graph.
///
/// The path records every node it passes through, so appending to it can
/// check that the new link actually continues from where it ends instead of
/// jumping to an unrelated part of the graph, and any node along it can be
/// addressed without the links' endpoints on hand.  That makes it
/// self-contained enough to persist as is.
pub struct LinkPath<S: GChainSpec> {
    /// The nodes along the path from the base to the terminal, so always one
    /// more than there are links.
    nodes: Vec<NodeRef<S>>,
    links: Vec<LinkRef<S>>,
}

impl<S: GChainSpec> LinkPath<S> {
    /// Creates a path rooted at a node with no links traversed yet.
    pub fn new_at(base_node: NodeRef<S>) -> Self {
        Self {
            nodes: vec![base_node],
            links: Vec::new(),
        }
    }

    /// Lays a path down from a base node through steps of a link and the
    /// node it reaches, trusting that each link departs from the node before
    /// it.
    pub fn from_steps(
        base_node: NodeRef<S>,
        steps: impl IntoIterator<Item = (LinkRef<S>, NodeRef<S>)>,
    ) -> Self {
        let mut path = Self::new_at(base_node);
        for (lref, target) in steps {
            path.links.push(lref);
            path.nodes.push(target);
        }
        path
    }

    /// The node the path starts from.  Traversing `links` in order starting
    /// here reaches the path's terminal node.
    pub fn base_node(&self) -> &NodeRef<S> {
        &self.nodes[0]
    }

    /// The node the path currently ends at.
    pub fn terminal_node(&self) -> &NodeRef<S> {
        self.nodes
            .last()
            .expect("gchain: path always has its base node")
    }

    /// The nodes along the path, from the base to the terminal.
    pub fn nodes(&self) -> &[NodeRef<S>] {
        &self.nodes
    }

    /// The links making up the path, in traversal order.
    pub fn links(&self) -> &[LinkRef<S>] {
        &self.links
    }

    /// Each link with the node it reaches, in traversal order.
    pub fn steps(&self) -> impl Iterator<Item = (&LinkRef<S>, &NodeRef<S>)> {
        self.links.iter().zip(&self.nodes[1..])
    }

    pub fn len(&self) -> usize {
        self.links.len()
    }

    pub fn is_empty(&self) -> bool {
        self.links.is_empty()
    }

    /// A node's position along the path, counting the base as zero.
    pub fn get_node_index(&self, node: &NodeRef<S>) -> Option<usize> {
        self.nodes.iter().position(|n| n == node)
    }

    /// The part of the path between two of its node positions.
    ///
    /// # Panics
    ///
    /// If `from > to` or `to` is past the terminal node.
    pub fn slice(&self, from: usize, to: usize) -> Self {
        assert!(
            from <= to && to < self.nodes.len(),
            "gchain: path slice out of range"
        );
        Self {
            nodes: self.nodes[from..=to].to_vec(),
            links: self.links[from..to].to_vec(),
        }
    }

    /// Attempts to add a link onto the end of the path.
    ///
    /// Returns `false` without modifying the path if the link doesn't depart
    /// from the node the path currently ends at.
    pub fn try_push_link(&mut self, lref: LinkRef<S>, endpoints: &LinkEndpoints<S>) -> bool {
        if endpoints.origin() != self.terminal_node() {
            return false;
        }

        self.links.push(lref);
        self.nodes.push(endpoints.target().clone());
        true
    }

    /// Attempts to add a whole path onto the end of this one.
    ///
    /// Returns `false` without modifying the path if the other doesn't start
    /// from the node this one currently ends at.
    pub fn try_extend(&mut self, other: &Self) -> bool {
        if other.base_node() != self.terminal_node() {
            return false;
        }

        self.links.extend(other.links.iter().cloned());
        self.nodes.extend(other.nodes[1..].iter().cloned());
        true
    }

    /// Checks that the path is shaped like one: a base node followed by one
    /// node per link.
    ///
    /// Paths built through this type always are; this is for one read back
    /// from somewhere it was persisted.
    pub fn sanity_check(&self) -> bool {
        self.nodes.len() == self.links.len() + 1
    }

    /// Checks against the provider that each link really connects the nodes
    /// on either side of it.
    ///
    /// Returns `Ok(false)` if the path is misshapen, a link is unknown to the
    /// provider, or a link sits somewhere else in the graph.
    pub fn sanity_check_against(
        &self,
        provider: &impl ChainProvider<Spec = S>,
    ) -> Result<bool, ProviderError> {
        if !self.sanity_check() {
            return Ok(false);
        }

        for (idx, lref) in self.links.iter().enumerate() {
            let Some(endpoints) = provider.fetch_link_endpoints(lref)? else {
                return Ok(false);
            };
            if *endpoints.origin() != self.nodes[idx] || *endpoints.target() != self.nodes[idx + 1]
            {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

// Implemented by hand for the same reason as on `LinkEndpoints`.
impl<S: GChainSpec> Clone for LinkPath<S> {
    fn clone(&self) -> Self {
        Self {
            nodes: self.nodes.clone(),
            links: self.links.clone(),
        }
    }
}

impl<S: GChainSpec> Debug for LinkPath<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LinkPath")
            .field("nodes", &self.nodes)
            .field("links", &self.links)
            .finish()
    }
}

impl<S: GChainSpec> PartialEq for LinkPath<S> {
    fn eq(&self, other: &Self) -> bool {
        self.nodes == other.nodes && self.links == other.links
    }
}

impl<S: GChainSpec> Eq for LinkPath<S> {}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
    struct TestRef(u8);

    impl GNodeRef for TestRef {}
    impl GLinkRef for TestRef {}

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct TestLink(u8);

    impl GLinkHeader for TestLink {}

    impl GLink for TestLink {
        fn check_structurally_consistent(&self) -> bool {
            true
        }
    }

    struct TestSpec;

    impl GChainSpec for TestSpec {
        type NodeRef = TestRef;
        type LinkRef = TestRef;
        type LinkHeader = TestLink;
        type Link = TestLink;

        fn get_header_ref(lh: &TestLink) -> TestRef {
            TestRef(lh.0)
        }

        fn get_header_canonical_prev(_lh: &TestLink) -> Option<TestRef> {
            None
        }
    }

    fn endpoints(origin: u8, target: u8) -> LinkEndpoints<TestSpec> {
        LinkEndpoints::new(TestRef(origin), TestRef(target))
    }

    #[test]
    fn test_new_path_terminates_at_its_base() {
        let path = LinkPath::<TestSpec>::new_at(TestRef(1));
        assert_eq!(path.base_node(), &TestRef(1));
        assert_eq!(path.terminal_node(), &TestRef(1));
        assert!(path.is_empty());
    }

    #[test]
    fn test_push_link_continuing_path_advances_terminal() {
        let mut path = LinkPath::<TestSpec>::new_at(TestRef(1));

        assert!(path.try_push_link(TestRef(10), &endpoints(1, 2)));
        assert!(path.try_push_link(TestRef(11), &endpoints(2, 3)));

        assert_eq!(path.links(), &[TestRef(10), TestRef(11)]);
        assert_eq!(path.terminal_node(), &TestRef(3));
        assert_eq!(path.base_node(), &TestRef(1));
        assert_eq!(path.len(), 2);
    }

    /// A link that departs from somewhere else entirely would silently splice
    /// two unrelated parts of the graph together.
    #[test]
    fn test_push_link_from_other_node_is_rejected() {
        let mut path = LinkPath::<TestSpec>::new_at(TestRef(1));
        assert!(path.try_push_link(TestRef(10), &endpoints(1, 2)));

        assert!(!path.try_push_link(TestRef(11), &endpoints(7, 8)));

        // The rejected link left no trace.
        assert_eq!(path.links(), &[TestRef(10)]);
        assert_eq!(path.terminal_node(), &TestRef(2));
    }

    /// Two different links reaching the same node is the whole point of the
    /// graph model, so either is acceptable from a given terminal.
    #[test]
    fn test_converging_links_are_both_acceptable() {
        let mut via_block = LinkPath::<TestSpec>::new_at(TestRef(1));
        let mut via_ckpt = LinkPath::<TestSpec>::new_at(TestRef(1));

        assert!(via_block.try_push_link(TestRef(10), &endpoints(1, 2)));
        assert!(via_block.try_push_link(TestRef(11), &endpoints(2, 9)));
        assert!(via_ckpt.try_push_link(TestRef(20), &endpoints(1, 9)));

        assert_eq!(via_block.terminal_node(), via_ckpt.terminal_node());
        assert_ne!(via_block.links(), via_ckpt.links());
    }

    #[test]
    fn test_path_records_every_node_it_passes() {
        let path = LinkPath::<TestSpec>::from_steps(
            TestRef(1),
            [(TestRef(10), TestRef(2)), (TestRef(11), TestRef(3))],
        );

        assert_eq!(path.nodes(), &[TestRef(1), TestRef(2), TestRef(3)]);
        assert_eq!(path.get_node_index(&TestRef(2)), Some(1));
        assert_eq!(path.get_node_index(&TestRef(9)), None);
        let steps: Vec<_> = path.steps().map(|(l, n)| (*l, *n)).collect();
        assert_eq!(
            steps,
            vec![(TestRef(10), TestRef(2)), (TestRef(11), TestRef(3))]
        );

        let tail = path.slice(1, 2);
        assert_eq!(tail.base_node(), &TestRef(2));
        assert_eq!(tail.links(), &[TestRef(11)]);
        let empty = path.slice(1, 1);
        assert!(empty.is_empty());
        assert_eq!(empty.terminal_node(), &TestRef(2));

        let mut head = path.slice(0, 1);
        assert!(head.try_extend(&tail));
        assert_eq!(head, path);
        assert!(!head.try_extend(&tail));
    }

    /// Endpoints keyed by link, standing in for a chain provider.
    struct MapProvider(HashMap<TestRef, LinkEndpoints<TestSpec>>);

    impl ChainProvider for MapProvider {
        type Spec = TestSpec;

        fn fetch_link_header(&self, lref: &TestRef) -> Result<Option<TestLink>, ProviderError> {
            Ok(self.0.contains_key(lref).then_some(TestLink(lref.0)))
        }

        fn fetch_link(&self, lref: &TestRef) -> Result<Option<TestLink>, ProviderError> {
            self.fetch_link_header(lref)
        }

        fn fetch_link_endpoints(
            &self,
            lref: &TestRef,
        ) -> Result<Option<LinkEndpoints<TestSpec>>, ProviderError> {
            Ok(self.0.get(lref).cloned())
        }

        fn fetch_forward_links(&self, nref: &TestRef) -> Result<Vec<TestRef>, ProviderError> {
            Ok(self
                .0
                .iter()
                .filter(|(_, e)| e.origin() == nref)
                .map(|(l, _)| *l)
                .collect())
        }

        fn fetch_backward_links(&self, nref: &TestRef) -> Result<Vec<TestRef>, ProviderError> {
            Ok(self
                .0
                .iter()
                .filter(|(_, e)| e.target() == nref)
                .map(|(l, _)| *l)
                .collect())
        }
    }

    #[test]
    fn test_sanity_checks_catch_misshapen_and_misplaced_paths() {
        let provider = MapProvider(HashMap::from([
            (TestRef(10), endpoints(1, 2)),
            (TestRef(11), endpoints(2, 3)),
        ]));
        let steps = |lrefs: &[u8], nodes: &[u8]| LinkPath::<TestSpec> {
            nodes: nodes.iter().map(|n| TestRef(*n)).collect(),
            links: lrefs.iter().map(|l| TestRef(*l)).collect(),
        };

        let good = steps(&[10, 11], &[1, 2, 3]);
        assert!(good.sanity_check());
        assert!(good.sanity_check_against(&provider).expect("test: check"));

        let misshapen = steps(&[10, 11], &[1, 2]);
        assert!(!misshapen.sanity_check());
        assert!(
            !misshapen
                .sanity_check_against(&provider)
                .expect("test: check")
        );

        let misplaced = steps(&[11, 10], &[1, 2, 3]);
        assert!(misplaced.sanity_check());
        assert!(
            !misplaced
                .sanity_check_against(&provider)
                .expect("test: check")
        );

        let unknown = steps(&[99], &[1, 2]);
        assert!(
            !unknown
                .sanity_check_against(&provider)
                .expect("test: check")
        );
    }
}
