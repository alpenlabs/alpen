use serde::{Deserialize, Serialize};

use crate::chain_spec::*;
use crate::processor::{ProcArtifact, ProcId};
use crate::version::{ProcVersion, RawProcVersion};

/// Description of an processor's execution for a link.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct ProcExecDesc<S: GChainSpec> {
    link_ref: LinkRef<S>,
    proc_id: ProcId,
}

impl<S: GChainSpec> ProcExecDesc<S> {
    pub fn new(link_ref: LinkRef<S>, proc_id: ProcId) -> Self {
        Self { link_ref, proc_id }
    }

    pub fn link_ref(&self) -> LinkRef<S> {
        self.link_ref
    }

    pub fn proc_id(&self) -> ProcId {
        self.proc_id
    }
}

/// Opaque data structure describing the results of processing a link.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ProcessorArtifactData {
    // short keys because serialization
    v: RawProcVersion,
    a: Vec<u8>,
}

impl ProcessorArtifactData {
    /// Returns the processor version used to produce this artifact.
    pub fn exec_version(&self) -> ProcVersion {
        self.v.into()
    }

    pub fn artifact(&self) -> &[u8] {
        &self.a
    }

    /// Attempts to decode the artifact data according to some concrete type.
    pub fn try_decode_artifact<A: ProcArtifact>(&self) -> anyhow::Result<A> {
        A::from_buf(self.artifact())
    }
}
