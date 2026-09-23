//! Persistence the executor relies on.

use strata_gchain_types::*;

/// A processed link as the store records it: the ref and the nodes it
/// connects.
///
/// The endpoints are what let an executor rebuild its graph of processed links
/// without consulting the chain provider.
pub struct LinkRecord<S: GChainSpec> {
    lref: LinkRef<S>,
    endpoints: LinkEndpoints<S>,
}

impl<S: GChainSpec> LinkRecord<S> {
    pub fn new(lref: LinkRef<S>, endpoints: LinkEndpoints<S>) -> Self {
        Self { lref, endpoints }
    }

    pub fn lref(&self) -> &LinkRef<S> {
        &self.lref
    }

    pub fn endpoints(&self) -> &LinkEndpoints<S> {
        &self.endpoints
    }

    pub fn into_parts(self) -> (LinkRef<S>, LinkEndpoints<S>) {
        (self.lref, self.endpoints)
    }
}

/// Storage for what an executor tracks between runs.
///
/// Artifacts are persisted by the executor rather than by the stages that
/// produced them, and alongside them the executor keeps enough about the graph
/// to pick up where it left off: which links it has processed and where they
/// sit, the path it has committed, and how far each stage has committed.  Each
/// method persists one of those independently; the executor orders its writes
/// so a crash between them leaves at worst something it knows how to discard.
///
/// Should only be used by one executor at a time.
pub trait ExecutorStore {
    /// The chain spec this store tracks.
    type Spec: GChainSpec;

    /// Persists the artifact a stage produced for a link, replacing any it had
    /// already stored for it.
    fn store_artifact(
        &self,
        lref: &LinkRef<Self::Spec>,
        proc_id: ProcId,
        data: &ProcessorArtifactData,
    ) -> Result<(), BoxedError>;

    /// Loads the artifact a stage previously produced for a link, if it's still
    /// stored.
    ///
    /// The caller checks [`ProcessorArtifactData::exec_version`] against the
    /// stage's current version before trusting the contents.
    fn load_artifact(
        &self,
        lref: &LinkRef<Self::Spec>,
        proc_id: ProcId,
    ) -> Result<Option<ProcessorArtifactData>, BoxedError>;

    /// Discards every stage's artifact for a link.
    fn discard_link_artifacts(&self, lref: &LinkRef<Self::Spec>) -> Result<(), BoxedError>;

    /// Records a link as processed.
    fn store_link(&self, record: &LinkRecord<Self::Spec>) -> Result<(), BoxedError>;

    /// Forgets a processed link.  Its artifacts are discarded separately.
    fn discard_link(&self, lref: &LinkRef<Self::Spec>) -> Result<(), BoxedError>;

    /// Loads every processed link.
    fn load_links(&self) -> Result<Vec<LinkRecord<Self::Spec>>, BoxedError>;

    /// Records the node a stage has committed its aggregated state up to.
    fn store_committed_node(
        &self,
        proc_id: ProcId,
        node: &NodeRef<Self::Spec>,
    ) -> Result<(), BoxedError>;

    /// Loads the node a stage has committed up to, if it has ever been
    /// initialized.
    fn load_committed_node(
        &self,
        proc_id: ProcId,
    ) -> Result<Option<NodeRef<Self::Spec>>, BoxedError>;

    /// Records the path the pipeline has committed: the links from the oldest
    /// node it can still roll back to, in traversal order, ending at the
    /// committed node.
    fn store_committed_path(&self, path: &PathDesc<Self::Spec>) -> Result<(), BoxedError>;

    /// Loads the committed path, if the pipeline has ever been initialized.
    fn load_committed_path(&self) -> Result<Option<PathDesc<Self::Spec>>, BoxedError>;
}
