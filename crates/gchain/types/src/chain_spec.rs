//! High-level definitions for a generic chain.
//!
//! The gchain system is based around the idea of a "chain graph".  This system
//! represents chain history as a directed acyclic graph, not a traditional
//! chain tree.  This is with the goal of unifying checkpoint sync and full sync.
//! There's multiple possible valid paths that can be taken through the chain
//! graph, with the path taken dictated by whatever sync logic is driving the
//! executor.
//!
//! The chain provider exposes the topology of the chain, but is told how to
//! traverse it instead of figuring that out itself.  The sync mode in control
//! of the gchain executor makes the decisions about that.
//!
//! Nodes correspond to "at rest" states.  Blocks and checkpoints are different
//! types of state transitions forming links between nodes.

use std::fmt::Debug;
use std::hash::Hash;

pub type NodeRef<S: GChainSpec> = <S as GChainSpec>::NodeRef;
pub type Node<S: GChainSpec> = <S as GChainSpec>::Node;
pub type LinkRef<S: GChainSpec> = <S as GChainSpec>::LinkRef;
pub type LinkHeader<S: GChainSpec> = <S as GChainSpec>::LinkHeader;
pub type Link<S: GChainSpec> = <S as GChainSpec>::Link;

/// Toplevel trait describing a chain.
pub trait GChainSpec: 'static {
    /// The chain's node ref type.
    type NodeRef: GNodeRef;

    /// The chain's node type.
    type Node: GNode;

    /// The chain's link ref type.
    type LinkRef: GLinkRef;

    /// The chain's node's header type.
    type LinkHeader: GLinkHeader;

    /// The chain's link type.
    type Link: GLink;

    /// Gets or computes the ref that points to this header.
    fn get_header_ref(lh: &Self::LinkHeader) -> Self::LinkRef;

    /// Checks if the link ref matches the header.
    ///
    /// Default impl just called `get_header_ref` and checks equality, but there
    /// may be more optimized impls.
    fn check_ref_match_header(lref: &Self::LinkRef, lh: &Self::LinkHeader) -> bool {
        Self::get_header_ref(lh) == *lref
    }

    /// Gets the in-protocol canonical previous link ref from the header (such
    /// as a "parent block"), if there is one.  There may be other completely
    /// valid links reaching the same origin node, but those may be across sync
    /// modes.
    ///
    /// This is what lets us walk a header chain backwards without consulting a
    /// provider, which the sync driver uses to assemble a candidate path before
    /// it has fetched any link bodies.
    fn get_header_canonical_prev(lh: &Self::LinkHeader) -> Option<Self::LinkRef>;
}

/// Describes a reference to a gchain node.
pub trait GNodeRef: Clone + Debug + Eq + PartialEq + Ord + PartialOrd + Hash {}

/// A node in the chain.
///
/// This is an "at rest" state that is one end of a state transitions described by links.
// TODO do we still need this?
pub trait GNode: Clone {
    // TODO
}

/// A link between two nodes.
pub trait GLinkRef: Clone + Debug + Eq + PartialEq + Ord + PartialOrd + Hash {
    // TODO
}

/// A header for a link in the chain.
///
/// These are meant to be small enough that we can keep many of them in memory
/// at once.  Commits to the full link data.
pub trait GLinkHeader: Clone + Debug + Eq + PartialEq {}

/// A link in the chain.
///
/// This is the full authoritative information needed to perform executor
/// processes on the state transition represented by the link.
pub trait GLink: Clone {
    /// Checks if the node is internally consistent.  Ie. that commitment(s) to
    /// the body in the header actually match the body.
    fn check_structurally_consistent(&self) -> bool;
}

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
#[derive(Clone, Debug, Eq, PartialEq)]
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

/// Describes a path through the node graph.
///
/// The path always knows the node it currently ends at, so appending to it can
/// check that the new link actually continues from there instead of jumping to
/// an unrelated part of the graph.
pub struct LinkPath<S: GChainSpec> {
    base_node: NodeRef<S>,
    terminal_node: NodeRef<S>,
    links: Vec<LinkRef<S>>,
}

impl<S: GChainSpec> LinkPath<S> {
    /// Creates a path rooted at a node with no links traversed yet.
    pub fn new_at(base_node: NodeRef<S>) -> Self {
        Self {
            terminal_node: base_node.clone(),
            base_node,
            links: Vec::new(),
        }
    }

    /// The node the path starts from.  Traversing `links` in order starting
    /// here reaches the path's terminal node.
    pub fn base_node(&self) -> &NodeRef<S> {
        &self.base_node
    }

    /// The node the path currently ends at.
    pub fn terminal_node(&self) -> &NodeRef<S> {
        &self.terminal_node
    }

    /// The links making up the path, in traversal order.
    pub fn links(&self) -> &[LinkRef<S>] {
        &self.links
    }

    pub fn len(&self) -> usize {
        self.links.len()
    }

    pub fn is_empty(&self) -> bool {
        self.links.is_empty()
    }

    /// Attempts to add a link onto the end of the path.
    ///
    /// Returns `false` without modifying the path if the link doesn't depart
    /// from the node the path currently ends at.
    pub fn try_push_link(&mut self, lref: LinkRef<S>, endpoints: &LinkEndpoints<S>) -> bool {
        if *endpoints.origin() != self.terminal_node {
            return false;
        }

        self.links.push(lref);
        self.terminal_node = endpoints.target().clone();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
    struct TestRef(u8);

    impl GNodeRef for TestRef {}
    impl GLinkRef for TestRef {}

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct TestLink(u8);

    impl GNode for TestLink {}
    impl GLinkHeader for TestLink {}

    impl GLink for TestLink {
        fn check_structurally_consistent(&self) -> bool {
            true
        }
    }

    struct TestSpec;

    impl GChainSpec for TestSpec {
        type NodeRef = TestRef;
        type Node = TestLink;
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
}
