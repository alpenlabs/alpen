//! High-level definitions for a generic chain.
//!
//! The gchain system is based around the idea of a "chain graph".  This system
//! represents chain history as a directed acyclic graph, not a traditional
//! chain tree.  This is with the goal of unifying checkpoint sync and full sync.
//! There's multiple possible paths through the chain tree.
//!
//! The chain provider exposes the topology of the chain, but is told how to
//! traverse it instead of figuring that out itself.  The sync mode in control
//! of the gchain executor makes the decisions about that.

use std::collections::*;
use std::fmt::Debug;
use std::hash::Hash;

pub type NodeRef<S: GChainSpec> = <S as GChainSpec>::NodeRef;
pub type NodeHeader<S: GChainSpec> = <S as GChainSpec>::NodeHeader;
pub type Node<S: GChainSpec> = <S as GChainSpec>::Node;
pub type NodeTable<S: GChainSpec> = HashMap<NodeRef<S>, Node<S>>;

/// Toplevel trait describing a chain.
pub trait GChainSpec {
    /// The chain's node ref type.
    type NodeRef: GNodeRef;

    /// The chain's node's header type.
    type NodeHeader: GNodeHeader;

    /// The chain's node type.
    type Node: GNode;

    /// Gets or computes the ref that points to this header.
    fn get_header_ref(nh: &Self::NodeHeader) -> Self::NodeRef;

    /// Checks if the node ref matches the header.
    ///
    /// Default impl just called `get_header_ref` and checks equality, but there
    /// may be more optimized impls.
    fn check_ref_match_header(nref: &Self::NodeRef, nh: &Self::NodeHeader) -> bool {
        Self::get_header_ref(nh) == *nref
    }

    /// Gets the header of a node.
    fn get_node_header(n: &Self::Node) -> Self::NodeHeader;

    /// Gets the in-protocol canonical previous node ref from the header (such
    /// as a "parent block"), if there is one.  There may be other completely
    /// valid previous nodes, but this may be across sync modes.
    // TODO(trey): do we need this fn?
    fn get_header_canonical_prev(nh: &Self::NodeHeader) -> Option<Self::NodeRef>;
}

/// Describes a reference to a gchain node, which can be used to look up the
/// corresponding node/header.
pub trait GNodeRef: Copy + Clone + Debug + Eq + PartialEq + Ord + PartialOrd + Hash {}

/// A header for a node in the chain.
///
/// These are meant to be small enough that we can keep many of them in memory
/// at once.  Commits to the full node body.
pub trait GNodeHeader: Clone + Debug + Eq + PartialEq {}

/// A node in the chain.
///
/// This is something like a block or a checkpoint.
pub trait GNode: Clone {
    /// Checks if the node is internally consistent.  Ie. that commitment(s) to
    /// the body in the header actually match the body.
    fn check_structurally_consistent(&self) -> bool;
}

/// Describes a link in the node chain, where a node points back to the previous
/// node we processed, if one exists.
///
/// A node might have multiple possible previous nodes depending on the path we
/// took traversing the tree, so there may be multiple valid [`NodeLink`]s with
/// some `node` value, varying in their `prev` value.
#[derive(Copy, Clone, Debug)]
pub struct NodeLink<S: GChainSpec> {
    node: S::NodeRef,
    prev: Option<S::NodeRef>,
}

impl<S: GChainSpec> NodeLink<S> {
    pub fn new(node: S::NodeRef, prev: Option<S::NodeRef>) -> Self {
        Self { node, prev }
    }

    pub fn new_step(node: S::NodeRef, prev: S::NodeRef) -> Self {
        Self::new(node, Some(prev))
    }

    pub fn new_base(base_node: S::NodeRef) -> Self {
        Self::new(base_node, None)
    }

    pub fn node(&self) -> &S::NodeRef {
        &self.node
    }

    pub fn prev(&self) -> Option<&S::NodeRef> {
        self.prev.as_ref()
    }

    /// Gets if the link has a previous node.
    pub fn has_prev(&self) -> bool {
        self.prev().is_some()
    }

    /// Gets the previously processed node.
    ///
    /// # Panics
    ///
    /// If there was no previous node.
    pub fn expect_prev(&self) -> &S::NodeRef {
        self.prev().expect("gchain: link missing previous")
    }
}

/// Describes a path through the node graph.
pub struct NodePath<S: GChainSpec> {
    nodes: Vec<NodeRef<S>>,
}

impl<S: GChainSpec> NodePath<S> {
    pub fn new(nodes: Vec<NodeRef<S>>) -> Self {
        Self { nodes }
    }

    pub fn new_at(base: NodeRef<S>) -> Self {
        Self::new(vec![base])
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn start(&self) -> Option<&NodeRef<S>> {
        self.nodes.first()
    }

    pub fn end(&self) -> Option<&NodeRef<S>> {
        self.nodes.last()
    }

    /// If this path is empty, sets the starting node.  Returns if successful.
    pub fn try_set_base(&mut self, base: NodeRef<S>) -> bool {
        if self.is_empty() {
            self.nodes.push(base);
            true
        } else {
            false
        }
    }

    /// Attempts to push a link onto the graph, checking that the link's
    /// previous node matches the last node in the path.  Returns if successful.
    pub fn try_push_link(&mut self, link: &NodeLink<S>) -> bool {
        // It's kinda silly that this works in the `None` case, but it works!
        if self.end() == link.prev() {
            self.nodes.push(*link.node());
            true
        } else {
            false
        }
    }

    /// Iterates over the links from base to tip.
    pub fn iter_links(&mut self) -> impl Iterator<Item = NodeLink<S>> {
        self.nodes
            .windows(2)
            .map(|ns| NodeLink::new_step(ns[0], ns[1]))
    }

    pub fn pop_end(&mut self) -> Option<NodeRef<S>> {
        self.nodes.pop()
    }
}
