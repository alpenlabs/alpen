//! The OL state index stage.
//!
//! Re-runs the STF for each link the exec stage accepted, this time through
//! the indexer layer, to collect the index writes the link implies.  It
//! depends on the exec stage both for the link itself (to skip links exec
//! rejected) and along the path (to build the same pre-state exec used).

mod artifact;
mod block;
mod checkpoint;

use std::sync::Arc;

use strata_gchain_types::*;
use strata_ol_params::OLRuntimeParams;
use strata_ol_state_support_types::{BatchDiffState, IndexerWrites};

pub use self::artifact::OLIndexArtifact;
use self::block::index_block_link;
use self::checkpoint::index_checkpoint_link;
use crate::chain_spec::OLChainSpec;
use crate::errors::OLProcError;
use crate::exec::OLExecArtifact;
use crate::graph_types::{OLLink, OLLinkRef, OLStateNode};
use crate::providers::{L1ManifestProvider, OLIndexStore, OLStateStore};
use crate::step::{load_base_state, path_write_batches};

/// The OL state index stage.
pub struct OLIndexProc<S: OLStateStore, M: L1ManifestProvider, I: OLIndexStore> {
    /// The ID the exec stage is registered under, since this stage's deps are
    /// on it.
    exec_proc_id: ProcId,
    runtime_params: OLRuntimeParams,
    state_store: Arc<S>,
    manifest_provider: Arc<M>,
    index_store: I,
}

impl<S: OLStateStore, M: L1ManifestProvider, I: OLIndexStore> OLIndexProc<S, M, I> {
    pub const VERSION: u32 = 1;

    pub fn new(
        exec_proc_id: ProcId,
        runtime_params: OLRuntimeParams,
        state_store: Arc<S>,
        manifest_provider: Arc<M>,
        index_store: I,
    ) -> Self {
        Self {
            exec_proc_id,
            runtime_params,
            state_store,
            manifest_provider,
            index_store,
        }
    }

    /// The deps this stage has to be registered with for its context fetches
    /// to be honored.
    pub fn deps(&self) -> ProcDeps {
        ProcDeps::new(vec![self.exec_proc_id], vec![self.exec_proc_id])
    }
}

impl<S: OLStateStore, M: L1ManifestProvider, I: OLIndexStore> GChainProc for OLIndexProc<S, M, I> {
    type Spec = OLChainSpec;
    type Artifact = OLIndexArtifact;

    fn proc_version(&self) -> ProcVersion {
        Self::VERSION.into()
    }

    fn on_init(&self, _cur_node: &OLStateNode, _node: &OLStateNode) -> Result<(), ProcError> {
        Ok(())
    }

    fn process_link(
        &self,
        _lref: &OLLinkRef,
        link: &OLLink,
        ctx: &impl ProcContext<Self>,
    ) -> Result<OLIndexArtifact, ProcError> {
        let exec = ctx
            .get_cur_artifact::<OLExecArtifact>(self.exec_proc_id)
            .ok_or(ProcError::MissingDep(self.exec_proc_id))?;
        if exec.output().is_none() {
            return Ok(OLIndexArtifact::new(IndexerWrites::new()));
        }

        let path = ctx
            .get_path_artifacts::<OLExecArtifact>(self.exec_proc_id)
            .ok_or(OLProcError::MissingPathArtifacts(self.exec_proc_id))?;
        let base = load_base_state(self.state_store.as_ref(), path.base())?;
        let batches = path_write_batches(&path)?;
        let pre_state = BatchDiffState::new_over(&base, &batches);

        let res = match link {
            OLLink::Block(link) => index_block_link::<S>(&self.runtime_params, &pre_state, link),
            OLLink::Checkpoint(link) => index_checkpoint_link::<S>(
                &self.runtime_params,
                self.manifest_provider.as_ref(),
                &pre_state,
                link,
            ),
        };

        // The exec stage already accepted this link, so a rejection here is a
        // disagreement between the stages rather than a bad link, which is
        // how the `StepError` conversion reports it.
        Ok(OLIndexArtifact::new(res?))
    }

    fn commit_outputs(
        &self,
        path: &LinkPath<OLChainSpec>,
        outputs: &[Arc<OLIndexArtifact>],
    ) -> Result<(), ProcError> {
        for (lref, artifact) in path.links().iter().zip(outputs) {
            self.index_store
                .apply_index_writes(lref, artifact.writes())?;
        }
        Ok(())
    }

    fn uncommit_outputs(
        &self,
        path: &LinkPath<OLChainSpec>,
        _outputs: &[Arc<OLIndexArtifact>],
    ) -> Result<(), ProcError> {
        for lref in path.links().iter().rev() {
            self.index_store.revert_index_writes(lref)?;
        }
        Ok(())
    }

    fn preprune_artifact(
        &self,
        _lref: &OLLinkRef,
        _output: &OLIndexArtifact,
    ) -> Result<(), ProcError> {
        Ok(())
    }

    fn prune_state_upto(&self, _nref: &OLStateNode) -> Result<(), ProcError> {
        Ok(())
    }
}
