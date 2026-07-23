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

use std::collections::*;
use std::fmt::Debug;
use std::hash::Hash;

pub type NodeRef<S: GChainSpec> = <S as GChainSpec>::NodeRef;
pub type Node<S: GChainSpec> = <S as GChainSpec>::Node;
pub type LinkTable<S: GChainSpec> = HashMap<NodeRef<S>, Node<S>>;
pub type LinkRef<S: GChainSpec> = <S as GChainSpec>::LinkRef;
pub type LinkHeader<S: GChainSpec> = <S as GChainSpec>::LinkHeader;
pub type Link<S: GChainSpec> = <S as GChainSpec>::Link;

/// Toplevel trait describing a chain.
pub trait GChainSpec {
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
    fn get_header_ref(nh: &Self::LinkHeader) -> Self::LinkRef;

    /// Checks if the node ref matches the header.
    ///
    /// Default impl just called `get_header_ref` and checks equality, but there
    /// may be more optimized impls.
    fn check_ref_match_header(lref: &Self::LinkRef, lh: &Self::LinkHeader) -> bool {
        Self::get_header_ref(lh) == *lref
    }

    /// Gets the header of a node.
    fn get_link_header(n: &Self::LinkRef) -> Self::LinkHeader;

    /// Gets the in-protocol canonical previous node ref from the header (such
    /// as a "parent block"), if there is one.  There may be other completely
    /// valid previous nodes, but this may be across sync modes.
    // TODO(trey): do we need this fn?
    fn get_header_canonical_prev(nh: &Self::LinkHeader) -> Option<Self::LinkRef>;
}

/// Describes a reference to a gchain node.
///
///
pub trait GNodeRef: Copy + Clone + Debug + Eq + PartialEq + Ord + PartialOrd + Hash {}

/// A node in the chain.
///
/// This is an "at rest" state that is one end of a state transitions described by links.
// TODO do we still need this?
pub trait GNode: Clone {
    // TODO
}

/// A link between two nodes.
pub trait GLinkRef: Copy + Clone + Debug + Eq + PartialEq + Ord + PartialOrd + Hash {
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

/// Describes a path through the node graph.
pub struct LinkPath<S: GChainSpec> {
    base_node: NodeRef<S>,
    links: Vec<LinkRef<S>>,
}

impl<S: GChainSpec> LinkPath<S> {
    pub fn new(base_node: NodeRef<S>, links: Vec<LinkRef<S>>) -> Self {
        Self { base_node, links }
    }

    pub fn new_at(base_node: NodeRef<S>) -> Self {
        Self::new(base_node, Vec::new())
    }

    pub fn len(&self) -> usize {
        self.links.len()
    }

    pub fn is_empty(&self) -> bool {
        self.links.is_empty()
    }

    /// Attempts to add a link onto the end of the path, if valid.
    // TODO this is an iteration from an earlier simpler design, but right now
    // we don't have a way to authoritatively assert canonicality
    pub fn try_push_link(&mut self, lref: LinkRef<S>) -> bool {
        self.links.push(lref);
        true
    }
}
