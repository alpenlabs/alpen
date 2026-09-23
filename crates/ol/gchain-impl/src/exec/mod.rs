//! The OL execution stage.
//!
//! Runs the OL STF over each link and produces the resulting write batch.  A
//! block link is verified by executing it; a checkpoint link is reconstructed
//! from its DA payload.  Either way the artifact is the diff from the origin
//! node to the target node, and committing a path means deriving the state at
//! its end from the state at its start plus those diffs.

mod artifact;
mod block;
mod checkpoint;

use std::sync::Arc;

use strata_gchain_types::*;
use strata_ol_params::OLRuntimeParams;
use strata_ol_state_support_types::BatchDiffState;
use strata_ol_state_types::IStateAccessor;

pub use self::artifact::{OLExecArtifact, OLExecOutput};
use self::block::process_block_link;
use self::checkpoint::process_checkpoint_link;
use crate::chain_spec::OLChainSpec;
use crate::errors::OLProcError;
use crate::graph_types::{OLLink, OLLinkRef, OLStateNode};
use crate::providers::{L1ManifestProvider, OLStateStore};
use crate::step::{StepError, load_base_state, path_write_batches};

/// The OL execution stage.
pub struct OLExecProc<S: OLStateStore, M: L1ManifestProvider> {
    runtime_params: OLRuntimeParams,
    state_store: Arc<S>,
    manifest_provider: Arc<M>,
}

impl<S: OLStateStore, M: L1ManifestProvider> OLExecProc<S, M> {
    pub const VERSION: u32 = 1;

    pub fn new(
        runtime_params: OLRuntimeParams,
        state_store: Arc<S>,
        manifest_provider: Arc<M>,
    ) -> Self {
        Self {
            runtime_params,
            state_store,
            manifest_provider,
        }
    }
}

impl<S: OLStateStore, M: L1ManifestProvider> GChainProc for OLExecProc<S, M> {
    type Spec = OLChainSpec;
    type Artifact = OLExecArtifact;

    fn proc_version(&self) -> ProcVersion {
        Self::VERSION.into()
    }

    fn on_init(&self, cur_node: &OLStateNode, _node: &OLStateNode) -> Result<(), ProcError> {
        // The committed state is the whole aggregated state, so there's nothing
        // to set up beyond making sure it's there to build on.
        load_base_state(self.state_store.as_ref(), cur_node).map(|_| ())
    }

    fn process_link(
        &self,
        _lref: &OLLinkRef,
        link: &OLLink,
        ctx: &impl ProcContext<Self>,
    ) -> Result<OLExecArtifact, ProcError> {
        let own_id = ctx.proc_id();
        let path = ctx
            .get_path_artifacts::<OLExecArtifact>(own_id)
            .ok_or(OLProcError::MissingPathArtifacts(own_id))?;
        let base = load_base_state(self.state_store.as_ref(), path.base())?;
        let batches = path_write_batches(&path)?;
        let pre_state = BatchDiffState::new_over(&base, &batches);

        let res = match link {
            OLLink::Block(link) => process_block_link::<S>(&self.runtime_params, &pre_state, link),
            OLLink::Checkpoint(link) => process_checkpoint_link::<S>(
                &self.runtime_params,
                self.manifest_provider.as_ref(),
                &pre_state,
                link,
            ),
        };

        match res {
            Ok(output) => Ok(OLExecArtifact::Valid(output)),
            Err(StepError::Invalid(reason)) => Ok(OLExecArtifact::Invalid(reason.to_string())),
            Err(err) => Err(err.into()),
        }
    }

    fn commit_outputs(
        &self,
        path: &LinkPath<OLChainSpec>,
        outputs: &[Arc<OLExecArtifact>],
    ) -> Result<(), ProcError> {
        let mut diffs = Vec::with_capacity(outputs.len());
        for (lref, artifact) in path.links().iter().zip(outputs) {
            let output = artifact
                .output()
                .ok_or(OLProcError::InvalidLinkOnPath(*lref))?;
            diffs.push(output.write_batch());

            if matches!(lref, OLLinkRef::Checkpoint(_)) {
                self.state_store.store_terminal_header(output.header())?;
            }
        }

        // The node names the root it expects; a mismatch means the artifacts
        // don't actually lead there, which is a bug rather than a bad link.
        let base_node = path.base_node();
        let terminal = path.terminal_node();
        let base = load_base_state(self.state_store.as_ref(), base_node)?;
        let got = BatchDiffState::new_over(&base, &diffs)
            .compute_state_root()
            .map_err(OLProcError::State)?;
        if got != *terminal.state_root() {
            return Err(OLProcError::StateRootMismatch {
                node: *terminal,
                got,
            }
            .into());
        }

        self.state_store.derive_state(base_node, terminal, &diffs)
    }

    fn uncommit_outputs(
        &self,
        path: &LinkPath<OLChainSpec>,
        _outputs: &[Arc<OLExecArtifact>],
    ) -> Result<(), ProcError> {
        // Write batches aren't reversible, so rolling back means falling back
        // to the state at the path's base, which has to still be there.
        load_base_state(self.state_store.as_ref(), path.base_node())?;
        self.state_store.delete_state(path.terminal_node())
    }

    fn preprune_artifact(
        &self,
        _lref: &OLLinkRef,
        _output: &OLExecArtifact,
    ) -> Result<(), ProcError> {
        Ok(())
    }

    fn prune_state_upto(&self, nref: &OLStateNode) -> Result<(), ProcError> {
        self.state_store.prune_states_before(nref)
    }
}
