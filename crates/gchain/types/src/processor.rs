//! Describes gchain processors.
//!
//! The general concept is that processors are applied to a node in stages by
//! the gchain processor executor.  The general principle is that there's
//! moderately-sized outputs a processor produces from each node.  The processor
//! maintains some abstract aggregated base state that it may access in order to
//! produce gchain output.
//!
//! The happy path looks like this:
//! 1. The executor picks a new node to process.
//! 2. The executor calls th `process_node` fn to produce an output.  The output is persisted.
//! 4. Some time later, the executor decides a node is committed.
//! 5. The executor calls `commit_node_output`.
//! 6.
//!
//! The key idea is that the aggregated state is managed by the processor and is
//! updated infrequently.  The by-node state is managed by the executor and is
//! updated on the fly as needed.  The executor tracks which processors have
//! been called on which nodes and orchestrates execution to bring them all
//! forwards up to the tip.

use std::any::{Any, TypeId};
use std::collections::*;
use std::fmt::{self, Debug, Display};
use std::str::{self, FromStr};
use std::sync::Arc;

use crate::chain_spec::{GChainSpec, Node, NodeLink, NodePath, NodeRef};

const PROC_ID_LEN: usize = 8;

/// ID used to refer to a registered processor stage.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct ProcId([u8; 8]);

impl FromStr for ProcId {
    type Err = ();

    // TODO(trey): make this a real error
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if !s.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err(());
        }

        let sb = s.as_bytes();
        if sb.len() > PROC_ID_LEN {
            return Err(());
        }

        let mut inner = [0; PROC_ID_LEN];
        inner.copy_from_slice(sb);
        Ok(Self(inner))
    }
}

impl AsRef<str> for ProcId {
    fn as_ref(&self) -> &str {
        let idx = self
            .0
            .iter()
            .enumerate()
            .find_map(|(i, b)| (*b == 0).then(|| i))
            .unwrap_or(PROC_ID_LEN);
        unsafe { str::from_utf8_unchecked(&self.0[..idx]) }
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
/// should be indicated through [`ProcOutput::is_node_valid`].
pub trait GChainProc: Sized {
    /// The chain spec this gchain proc is defined for.
    type Spec: GChainSpec;

    /// An incremental container for the output of running a processor on a node.
    type NodeProcOutput: ProcOutput;

    /// Called when the processor is first initialized.
    ///
    /// This only ever happens once.  Different processor stages may be inited
    /// on different first node links, such as when opening an older database
    /// with a newer client version (which added a new processor).
    fn on_init(&self, cur_node: &NodeRef<Self::Spec>, node: &Node<Self::Spec>);

    /// Processes a node and produces some output from the step.
    ///
    /// May fetch outputs declared in the deps (configured in the executor) from
    /// the provided context and use them in its processing.  The link indicates
    /// how we arrived at this node and so which data we can fetch from the
    /// context.
    fn process_node(
        &self,
        link: &NodeLink<Self::Spec>,
        node: &Node<Self::Spec>,
        ctx: &ProcContext<Self>,
    ) -> anyhow::Result<Self::NodeProcOutput>;

    /// Applies a path of processed outputs for multiple nodes into the
    /// aggregated state, as a single operation.
    ///
    /// The order of the outputs slice matches the order of nodes in the
    /// provided path.
    fn commit_node_outputs(
        &self,
        path: &NodePath<Self::Spec>,
        outputs: &[Arc<Self::NodeProcOutput>],
    ) -> anyhow::Result<()>;

    /// Rolls back the processed output of a node from the aggregated state (as
    /// a direct "undo" operation to `commit_node_outputs`), if possible, as a
    /// single operation.  The path provided is meant to be traversed "in
    /// reverse" compared to how it's traversed in `commit_node_outputs`.
    ///
    /// Will never be called with any node passed to `compact_state` or any node
    /// before it.
    fn uncommit_node_outputs(
        &self,
        path: &NodePath<Self::Spec>,
        outputs: &[Arc<Self::NodeProcOutput>],
    ) -> anyhow::Result<()>;

    /// Called by the executor before we discard an output (like one that's
    /// pruned) order to discard any auxiliary data that might exist.
    fn preprune_node_output(
        &self,
        link: &NodeLink<Self::Spec>,
        output: &Self::NodeProcOutput,
    ) -> anyhow::Result<()>;

    /// Called when we are sure we will never try to roll back to before a
    /// certain node so that we can perform cleanups and discard information we
    /// no longer need.
    ///
    /// The provided node will become the oldest node.
    fn prune_state_upto(self, nref: &NodeRef<Self::Spec>) -> anyhow::Result<()>;
}

/// Output from a processing stage on a node.
pub trait ProcOutput: Sync + Send + Any + 'static {
    /// Checks if the output indicates the node was valid, as far as the
    /// processor stage cares.  A layer processor stage may think that this is
    /// not true.
    ///
    /// Default impl assumes true, since a lot of processor stages may not
    /// actually be involved in node validation.
    fn is_node_valid(&self) -> bool {
        true
    }
}

/// Cached output from nodes that we've extracted and determined might be useful
/// for later proc stages.
pub struct OutputCache<S: GChainSpec> {
    nodes: HashMap<NodeRef<S>, BTreeMap<TypeId, Arc<dyn ProcOutput>>>,
}

impl<S: GChainSpec> OutputCache<S> {
    /// Gets the stored output from some processor for some node.
    pub fn get_proc_output_arc<O: ProcOutput>(
        &self,
        nref: &NodeRef<S>,
    ) -> Option<&Arc<dyn ProcOutput>> {
        self.nodes
            .get(nref)
            .and_then(|notbl| notbl.get(&TypeId::of::<O>()))
    }

    pub fn get_proc_output<O: ProcOutput>(&self, _nref: &NodeRef<S>) -> Option<&O> {
        // TODO(trey): need more complicated type hacks to make this work
        unimplemented!()
    }
}

/// Context from the executor passed into a processor.
pub struct ProcContext<P: GChainProc> {
    cached_outputs: OutputCache<P::Spec>,
}

impl<P: GChainProc> ProcContext<P> {
    // TODO
}

pub struct ProcHistory<P: GChainProc> {
    base: NodeRef<P::Spec>,
    steps: Vec<Arc<ProcStepOutput<P>>>,
}

impl<P: GChainProc> ProcHistory<P> {
    pub fn new(base: NodeRef<P::Spec>, steps: Vec<Arc<ProcStepOutput<P>>>) -> Self {
        Self { base, steps }
    }

    pub fn new_base(base: NodeRef<P::Spec>) -> Self {
        Self::new(base, Vec::new())
    }

    /// Pushes a step onto the end of this processing history.
    pub fn push_step(&mut self, outp: Arc<ProcStepOutput<P>>) {
        self.steps.push(outp);
    }

    pub fn base(&self) -> &NodeRef<P::Spec> {
        &self.base
    }

    pub fn steps(&self) -> &[Arc<ProcStepOutput<P>>] {
        &self.steps
    }

    /// Gets the last (most recent) processed node ref.
    pub fn last_node_ref(&self) -> &NodeRef<P::Spec> {
        self.steps.last().map(|o| &o.nref).unwrap_or(&self.base)
    }

    /// Gets the last (most recent) processed node link, if there is any.
    pub fn last_node_link(&self) -> Option<NodeLink<P::Spec>> {
        self.iter_links().next()
    }

    /// Produces the links for each of the steps in the processing history,
    /// starting from the most recent.
    pub fn iter_links(&self) -> impl Iterator<Item = NodeLink<P::Spec>> {
        // Walk the outputs newest-first, linking each to its predecessor and
        // stepping the oldest output back onto the base node.
        let n = self.steps.len();
        (0..n).rev().map(move |k| {
            let cur = self.steps[k].nref;
            let prev = if k > 0 {
                self.steps[k - 1].nref
            } else {
                self.base
            };
            NodeLink::new_step(cur, prev)
        })
    }
}

pub struct ProcStepOutput<P: GChainProc> {
    nref: NodeRef<P::Spec>,
    output: P::NodeProcOutput,
}

/// Describes the dependencies a processing stage has, so that we know which
/// ways we are allowed to run them in parallel.
#[derive(Clone, Debug)]
pub struct ProcDeps {
    /// Deps on other processors' output for the current node.
    cur_node: Vec<ProcId>,

    /// Deps on other processors' output for the previous node.
    prev_node: Vec<ProcId>,
}

impl ProcDeps {
    pub fn new(cur_node: Vec<ProcId>, prev_node: Vec<ProcId>) -> Self {
        Self {
            cur_node,
            prev_node,
        }
    }

    /// Deps on other processors' output for the current node.
    ///
    /// This limits how "widely" we can parallelize processing a single node.
    pub fn cur_node(&self) -> &[ProcId] {
        &self.cur_node
    }

    /// Deps on other processors' output for the previous node.
    ///
    /// This limits how "deeply" we can parallelize processing a stage across
    /// many nodes.  A processor that does core validation may depend on its own
    /// output from the previous node, so we have to process those in-order.
    /// But some indexing step might not care, so we can process many nodes in
    /// parallel.
    pub fn prev_node(&self) -> &[ProcId] {
        &self.prev_node
    }
}
