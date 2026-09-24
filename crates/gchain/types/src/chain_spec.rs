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
pub type LinkRef<S: GChainSpec> = <S as GChainSpec>::LinkRef;
pub type LinkHeader<S: GChainSpec> = <S as GChainSpec>::LinkHeader;
pub type Link<S: GChainSpec> = <S as GChainSpec>::Link;

/// Toplevel trait describing a chain.
pub trait GChainSpec: 'static {
    /// The chain's node ref type.
    ///
    /// Nodes are states at rest and have no data of their own beyond what the
    /// ref names; everything about a node is reconstructed from the links that
    /// reach it.
    type NodeRef: GNodeRef;

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
