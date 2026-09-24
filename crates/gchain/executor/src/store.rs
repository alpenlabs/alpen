//! Persistence the executor relies on.
//!
//! The executor is the only thing that talks to the store, and it keeps
//! little in memory beyond what's here: the tracking state is mirrored whole,
//! and artifacts are loaded per link as they're needed.  The tracking state is
//! written as one unit so its parts can never disagree; artifacts are written
//! only once every stage has accepted their link.
//!
//! A store should only be used by one executor at a time.

use strata_gchain_types::*;

use crate::tracking::TrackingState;

/// An artifact with the link and stage it belongs to, as the store keeps it.
pub struct ArtifactRecord<S: GChainSpec> {
    lref: LinkRef<S>,
    proc_id: ProcId,
    data: ProcessorArtifactData,
}

impl<S: GChainSpec> ArtifactRecord<S> {
    pub fn new(lref: LinkRef<S>, proc_id: ProcId, data: ProcessorArtifactData) -> Self {
        Self {
            lref,
            proc_id,
            data,
        }
    }

    pub fn lref(&self) -> &LinkRef<S> {
        &self.lref
    }

    pub fn proc_id(&self) -> ProcId {
        self.proc_id
    }

    pub fn data(&self) -> &ProcessorArtifactData {
        &self.data
    }

    pub fn into_parts(self) -> (LinkRef<S>, ProcId, ProcessorArtifactData) {
        (self.lref, self.proc_id, self.data)
    }
}

// Bounds fall on the ref type, not the marker spec type.
impl<S: GChainSpec> Clone for ArtifactRecord<S> {
    fn clone(&self) -> Self {
        Self {
            lref: self.lref.clone(),
            proc_id: self.proc_id,
            data: self.data.clone(),
        }
    }
}

/// Storage for what an executor tracks between runs.
///
/// Artifacts are persisted by the executor rather than by the stages that
/// produced them, and they're also the executor's record of which links it
/// has processed: a link is known iff some artifact is stored for it.
pub trait ExecutorStore {
    /// The chain spec this store tracks.
    type Spec: GChainSpec;

    /// Records where the pipeline stands, replacing what was there.
    fn store_tracking(&self, state: &TrackingState<Self::Spec>) -> Result<(), BoxedError>;

    /// Loads where the pipeline stands, if it has ever been initialized.
    fn load_tracking(&self) -> Result<Option<TrackingState<Self::Spec>>, BoxedError>;

    /// Persists the artifact a stage produced for a link, replacing any it had
    /// already stored for it.
    fn store_artifact(&self, record: &ArtifactRecord<Self::Spec>) -> Result<(), BoxedError>;

    /// Loads every artifact stored for a link.
    ///
    /// The executor checks each [`ProcessorArtifactData::exec_version`]
    /// against the stage's current version before trusting the contents.
    fn load_link_artifacts(
        &self,
        lref: &LinkRef<Self::Spec>,
    ) -> Result<Vec<ArtifactRecord<Self::Spec>>, BoxedError>;

    /// Whether any artifact is stored for a link, without loading them.
    fn has_link_artifacts(&self, lref: &LinkRef<Self::Spec>) -> Result<bool, BoxedError>;

    /// Discards every stage's artifact for a link, which also forgets the
    /// link.
    fn discard_link_artifacts(&self, lref: &LinkRef<Self::Spec>) -> Result<(), BoxedError>;
}
