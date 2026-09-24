//! Describes gchain processors.
//!
//! The general concept is that several different processors get applied to a
//! link in stages by a gchain processor executor.  These processors produce
//! some "artifact" from being applied to the link (such as a write batch),
//! which are only "moderately sized" and feasible to juggle many of in memory
//! (or recompute on the fly).  The processor itself maintains some abstract
//! aggregated base state that it may access in order to produce artifacts,
//! potentially through the lens of intermediate artifacts.
//!
//! The happy path looks like this:
//! 1. The executor picks a new node to process.
//! 2. The executor calls th `process_link` fn to produce an artifact.
//! 4. Some time later, the executor decides a (series of) link(s) is ready to be committed.
//! 5. The executor calls `commit_outputs`.
//!
//! A key idea is that the aggregated state is managed by the processor and is
//! updated infrequently.  The by-link state is managed by the executor and is
//! updated on the fly as needed.  The executor tracks which processors have
//! been called on which links and orchestrates execution to bring them all
//! forwards up to the tip.

use std::any::{Any, TypeId};
use std::fmt::{self, Debug, Display};
use std::str::{self, FromStr};
use std::sync::Arc;

use thiserror::Error as ThisError;

use crate::chain_spec::*;
use crate::errors::ProcError;
use crate::graph::*;
use crate::version::ProcVersion;

/// Maximum length of a [`ProcId`], in bytes.
pub const PROC_ID_LEN: usize = 8;

/// ID used to refer to a registered processor stage.
///
/// Always a non-empty, ASCII alphanumeric string of at most [`PROC_ID_LEN`]
/// bytes, zero-padded to a fixed width.  [`FromStr`] is the only constructor, so
/// that invariant holds for every value that exists.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct ProcId([u8; PROC_ID_LEN]);

/// Reasons a string isn't a well-formed [`ProcId`].
#[derive(Copy, Clone, Debug, Eq, PartialEq, ThisError)]
pub enum ProcIdParseError {
    /// An empty id would display as an empty string, making it impossible to
    /// tell apart from a missing value.
    #[error("proc id was empty")]
    Empty,

    #[error("proc id too long (expected at most {PROC_ID_LEN}, got {0})")]
    TooLong(usize),

    #[error("proc id contained a non-ASCII-alphanumeric character")]
    NotAlphanumeric,
}

impl FromStr for ProcId {
    type Err = ProcIdParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(ProcIdParseError::Empty);
        }

        if !s.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err(ProcIdParseError::NotAlphanumeric);
        }

        let sb = s.as_bytes();
        if sb.len() > PROC_ID_LEN {
            return Err(ProcIdParseError::TooLong(sb.len()));
        }

        let mut inner = [0; PROC_ID_LEN];
        inner[..sb.len()].copy_from_slice(sb);
        Ok(Self(inner))
    }
}

impl AsRef<str> for ProcId {
    fn as_ref(&self) -> &str {
        let idx = self.0.iter().position(|b| *b == 0).unwrap_or(PROC_ID_LEN);

        // Scanning at most 8 bytes isn't worth an `unsafe` that a future
        // constructor could silently invalidate.
        str::from_utf8(&self.0[..idx]).expect("gchain: ProcId is always ASCII")
    }
}

impl Debug for ProcId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ProcId({})", self.as_ref())
    }
}

impl Display for ProcId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_ref())
    }
}

/// Generic chain processor stage.
///
/// Error variants on result types should *ONLY* be used to indicate that the
/// processing *failed*, never that the node is invalid.  Nodes being invalid
/// should be indicated through [`ProcArtifact::is_link_valid`].
pub trait GChainProc: Sized + 'static {
    /// The chain spec this gchain proc is defined for.
    type Spec: GChainSpec;

    /// The incremental artifacts produced for the output of running on a link.
    type Artifact: ProcArtifact;

    /// The version of this stage's processing behavior.
    ///
    /// The executor records this alongside every artifact it persists, so that
    /// opening a database written by a client whose behavior has since changed
    /// can be detected and the affected links reprocessed.  Bump it whenever a
    /// change would produce a different artifact for the same link.
    fn proc_version(&self) -> ProcVersion;

    /// Called when the processor is first initialized, with the node its
    /// aggregated state is expected to be at.
    ///
    /// This only ever happens once, but this fn may be called multiple times
    /// (like if there's crashes on startup).  Different processor stages may be
    /// inited at different nodes, such as when opening an older database with a
    /// newer client version (which added a new processor).
    fn on_init(&self, cur_node: &NodeRef<Self::Spec>) -> Result<(), ProcError>;

    /// Processes a link and produces some output from the step.
    ///
    /// May fetch outputs declared in the deps (configured in the executor) from
    /// the provided context and use them in its processing.  The link indicates
    /// how we arrived at this node and so which data we can fetch from the
    /// context.
    fn process_link(
        &self,
        lref: &LinkRef<Self::Spec>,
        link: &Link<Self::Spec>,
        ctx: &impl ProcContext<Self>,
    ) -> Result<Self::Artifact, ProcError>;

    /// Applies a path of artifacts for processed links for multiple nodes into
    /// the aggregated state, as a single operation.
    ///
    /// The order of the outputs slice matches the order of nodes in the
    /// provided path.
    ///
    /// Must be idempotent: the executor records the stage's committed node
    /// only after this returns, so a crash in between has it called again with
    /// the same path on reopen.
    fn commit_outputs(
        &self,
        path: &LinkPath<Self::Spec>,
        outputs: &[Arc<Self::Artifact>],
    ) -> Result<(), ProcError>;

    /// Rolls back the artifacts of a set of links from the aggregated state (as
    /// a direct "undo" operation to `commit_outputs`), as a single operation.
    /// The path provided is meant to be traversed "in reverse" compared to how
    /// it's traversed in `commit_node_outputs`.
    ///
    /// Will never be called with any link passed to `compact_state` or any node
    /// before it.
    fn uncommit_outputs(
        &self,
        path: &LinkPath<Self::Spec>,
        outputs: &[Arc<Self::Artifact>],
    ) -> Result<(), ProcError>;

    /// Called by the executor before we discard an artifact (like one that's
    /// pruned) order to discard any auxiliary data that might exist.
    ///
    /// May be called multiple times for the same link/artifact.
    fn preprune_artifact(
        &self,
        lref: &LinkRef<Self::Spec>,
        output: &Self::Artifact,
    ) -> Result<(), ProcError>;

    /// Called when we are sure we will never try to roll back to before a
    /// certain node so that we can perform cleanups and discard information we
    /// no longer need.
    ///
    /// The provided node will become the oldest node.
    fn prune_state_upto(&self, nref: &NodeRef<Self::Spec>) -> Result<(), ProcError>;
}

/// Output from a processing stage on a link transition.
///
/// Artifacts are persisted by the executor, not by the stage that produced
/// them, so they have to round-trip through a buffer in both directions.
pub trait ProcArtifact: Sync + Send + Sized + 'static {
    /// Encodes the artifact so the executor can persist it.
    ///
    /// Must round-trip through [`ProcArtifact::from_buf`].
    fn to_buf(&self) -> Result<Vec<u8>, ProcError>;

    /// Attempts to decode a buf as the proc artifact.
    fn from_buf(buf: &[u8]) -> Result<Self, ProcError>;

    /// Checks if the output indicates the link transition was valid, as far as
    /// the processor stage cares.  A layer processor stage may be used to
    /// decide that a link is invalid and we should avoid doing more work on it
    /// (and preferentially take a different path through the graph).
    ///
    /// Default impl assumes true, since a lot of processor stages may not
    /// actually be involved in node validation.
    fn is_link_valid(&self) -> bool {
        true
    }
}

/// Dyn-compatible view of a [`ProcArtifact`].
///
/// The executor collects artifacts from every processor stage into shared
/// storage without knowing their concrete types, so it manipulates them through
/// this trait instead.  [`ProcArtifact`] itself can't serve this role because it
/// is `Sized` and has a constructor returning `Self`.
///
/// This is blanket impl'd for every [`ProcArtifact`], so processor stages never
/// implement it directly.
pub trait DynProcArtifact: Sync + Send + 'static {
    /// See [`ProcArtifact::is_link_valid`].
    fn is_link_valid(&self) -> bool;

    /// See [`ProcArtifact::to_buf`].
    ///
    /// Named apart from the [`ProcArtifact`] method so that calls on a concrete
    /// artifact type, which implements both traits, stay unambiguous.
    fn to_buf_dyn(&self) -> Result<Vec<u8>, ProcError>;

    /// Returns the type ID of the underlying concrete artifact type.
    fn artifact_type_id(&self) -> TypeId;

    /// Borrows as a handle that can be downcast back to the concrete artifact
    /// type.
    fn as_any(&self) -> &dyn Any;

    /// Converts to an owned handle that can be downcast back to the concrete
    /// artifact type.
    fn into_any_arc(self: Arc<Self>) -> Arc<dyn Any + Send + Sync>;
}

impl<A: ProcArtifact> DynProcArtifact for A {
    fn is_link_valid(&self) -> bool {
        <A as ProcArtifact>::is_link_valid(self)
    }

    fn to_buf_dyn(&self) -> Result<Vec<u8>, ProcError> {
        <A as ProcArtifact>::to_buf(self)
    }

    fn artifact_type_id(&self) -> TypeId {
        TypeId::of::<A>()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn into_any_arc(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

/// Describes the dependencies a processing stage has, so that we know which
/// ways we are allowed to run them in parallel.
#[derive(Clone, Debug)]
pub struct ProcDeps {
    /// Deps on other processors' output for the current link.
    cur_node: Vec<ProcId>,

    /// Deps on other processors' output along the path to the link's origin.
    prev_node: Vec<ProcId>,
}

impl ProcDeps {
    pub fn new(cur_node: Vec<ProcId>, prev_node: Vec<ProcId>) -> Self {
        Self {
            cur_node,
            prev_node,
        }
    }

    /// Deps on other processors' output for the current link.
    ///
    /// This limits how "widely" we can parallelize processing a single link:
    /// the named stages must have accepted the link before this stage runs on
    /// it.
    pub fn cur_node(&self) -> &[ProcId] {
        &self.cur_node
    }

    /// Deps on other processors' output along the path to the link's origin.
    ///
    /// This limits how "deeply" we can parallelize processing a stage across
    /// many links: the named stages must have artifacts for every link from
    /// their committed node to this link's origin, since that's what the state
    /// at the origin is reconstructed from.  A processor that does core
    /// validation depends on its own earlier output this way, so those links
    /// have to be processed in order.  But some indexing step might not care,
    /// so we can process many links in parallel.
    pub fn prev_node(&self) -> &[ProcId] {
        &self.prev_node
    }
}

/// Provider for context about a processing operation.
///
/// This exposes the artifacts other processor stages produced, so a stage can
/// build on their work instead of recomputing it.  A stage may only fetch
/// artifacts from stages it declared a dependency on in its [`ProcDeps`]; the
/// executor is free to treat any other fetch as missing, since it only
/// guarantees the ordering the declared deps imply.
///
/// The context is scoped to the link being processed.  The fetches correspond
/// to the two dep lists: the link currently being processed, and a path of
/// links reaching its origin node.  Which path that is when several converge
/// on the origin is the executor's choice, and a stage must not depend on it:
/// artifacts describe nodes, so every path from a stage's committed node to
/// the origin reconstructs the same state there.
pub trait ProcContext<P: GChainProc> {
    /// The ID the calling stage is registered under.
    ///
    /// Stages don't know their own ID otherwise, and a stage that builds on its
    /// own earlier artifacts needs it to fetch them.
    fn proc_id(&self) -> ProcId;

    /// Fetches the artifact another stage produced for the link currently being
    /// processed.
    ///
    /// Returns `None` if the stage produced no artifact for this link, or if
    /// the artifact isn't of type `A`.
    fn get_cur_artifact<A: ProcArtifact>(&self, proc_id: ProcId) -> Option<Arc<A>>;

    /// Fetches the artifacts a stage produced along an uncommitted path from
    /// its committed node to this link's origin node.
    ///
    /// This is what lets a stage reconstruct the state at the origin node
    /// without the executor having committed anything: its aggregated state is
    /// at the path's base, and the artifacts along the path are the diffs that
    /// carry it forward from there.
    ///
    /// Returns `None` if the stage has no artifact for some link on the path,
    /// or if any of them isn't of type `A`.  The path is empty when the origin
    /// node is the committed node itself.
    fn get_path_artifacts<A: ProcArtifact>(
        &self,
        proc_id: ProcId,
    ) -> Option<PathArtifacts<P::Spec, A>>;
}

/// Artifacts one stage produced along a path through the graph, in traversal
/// order.
///
/// The base is the node the path departs from, which for artifacts fetched
/// through [`ProcContext::get_path_artifacts`] is the stage's committed node.
pub struct PathArtifacts<S: GChainSpec, A> {
    base: NodeRef<S>,
    steps: Vec<(LinkRef<S>, Arc<A>)>,
}

impl<S: GChainSpec, A> PathArtifacts<S, A> {
    pub fn new(base: NodeRef<S>, steps: Vec<(LinkRef<S>, Arc<A>)>) -> Self {
        Self { base, steps }
    }

    /// The node the path departs from.
    pub fn base(&self) -> &NodeRef<S> {
        &self.base
    }

    /// The links along the path with their artifacts, in traversal order.
    pub fn steps(&self) -> &[(LinkRef<S>, Arc<A>)] {
        &self.steps
    }

    /// Iterates over just the artifacts, in traversal order.
    pub fn iter_artifacts(&self) -> impl Iterator<Item = &A> {
        self.steps.iter().map(|(_, a)| a.as_ref())
    }

    pub fn len(&self) -> usize {
        self.steps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;
    use std::sync::Arc;

    use super::{DynProcArtifact, PROC_ID_LEN, ProcArtifact, ProcId, ProcIdParseError};
    use crate::test_support::*;

    /// The executor checks link validity without knowing the concrete artifact
    /// type, so it has to work through the erased view.
    #[test]
    fn test_is_link_valid_visible_through_erasure() {
        let valid: Arc<dyn DynProcArtifact> = Arc::new(FlagArtifact(true));
        let invalid: Arc<dyn DynProcArtifact> = Arc::new(FlagArtifact(false));
        let indifferent: Arc<dyn DynProcArtifact> = Arc::new(CountArtifact(7));

        assert!(valid.is_link_valid());
        assert!(!invalid.is_link_valid());
        // Stages not involved in validation get the default.
        assert!(indifferent.is_link_valid());
    }

    /// Artifacts are stored type-erased, so the executor encodes them without
    /// knowing which stage produced them and downcasts them back for it.
    #[test]
    fn test_erased_artifact_encodes_and_downcasts() {
        let artifact: Arc<dyn DynProcArtifact> = Arc::new(CountArtifact(7));

        let buf = artifact.to_buf_dyn().expect("test: encode artifact");
        assert_eq!(
            CountArtifact::from_buf(&buf).expect("test: decode artifact"),
            CountArtifact(7)
        );

        assert_eq!(
            artifact.as_any().downcast_ref::<CountArtifact>(),
            Some(&CountArtifact(7))
        );
        assert!(artifact.as_any().downcast_ref::<FlagArtifact>().is_none());
        let owned = artifact
            .into_any_arc()
            .downcast::<CountArtifact>()
            .expect("test: downcast artifact");
        assert_eq!(*owned, CountArtifact(7));
    }

    #[test]
    fn test_parse_short_proc_id() {
        ProcId::from_str("foo").expect("test: parse ProcId");
    }

    #[test]
    fn test_parse_proc_id_roundtrips_through_str() {
        for s in ["a", "foo", "exactly8"] {
            let id = ProcId::from_str(s).expect("test: parse ProcId");
            assert_eq!(id.as_ref(), s);
            assert_eq!(id.to_string(), s);
        }
    }

    /// An empty id displays as an empty string, which is indistinguishable from
    /// a missing value, so it must not be constructible.
    #[test]
    fn test_parse_empty_proc_id_fails() {
        assert_eq!(ProcId::from_str(""), Err(ProcIdParseError::Empty));
    }

    #[test]
    fn test_parse_overlong_proc_id_fails() {
        let overlong = "a".repeat(PROC_ID_LEN + 1);
        assert_eq!(
            ProcId::from_str(&overlong),
            Err(ProcIdParseError::TooLong(PROC_ID_LEN + 1))
        );
    }

    #[test]
    fn test_parse_nonalphanumeric_proc_id_fails() {
        for s in ["foo_bar", "foo bar", "fo-o", "é"] {
            assert_eq!(
                ProcId::from_str(s),
                Err(ProcIdParseError::NotAlphanumeric),
                "test: expected {s:?} to be rejected"
            );
        }
    }
}
