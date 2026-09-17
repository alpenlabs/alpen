use serde::{Deserialize, Serialize};

use crate::chain_spec::*;
use crate::errors::ProcError;
use crate::processor::{ProcArtifact, ProcId};
use crate::version::{ProcVersion, RawProcVersion};

/// Description of an processor's execution for a link.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct ProcExecDesc<S: GChainSpec> {
    link_ref: LinkRef<S>,
    proc_id: ProcId,
}

impl<S: GChainSpec> ProcExecDesc<S> {
    pub fn new(link_ref: LinkRef<S>, proc_id: ProcId) -> Self {
        Self { link_ref, proc_id }
    }

    pub fn link_ref(&self) -> &LinkRef<S> {
        &self.link_ref
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
    /// Wraps an encoded artifact with the processor version that produced it.
    pub fn new(version: ProcVersion, artifact: Vec<u8>) -> Self {
        Self {
            v: version.into(),
            a: artifact,
        }
    }

    /// Encodes an artifact for persistence, tagging it with the version of the
    /// stage that produced it.
    pub fn from_artifact<A: ProcArtifact>(
        version: ProcVersion,
        artifact: &A,
    ) -> Result<Self, ProcError> {
        Ok(Self::new(version, artifact.to_buf()?))
    }

    /// Returns the processor version used to produce this artifact.
    pub fn exec_version(&self) -> ProcVersion {
        self.v.into()
    }

    pub fn artifact(&self) -> &[u8] {
        &self.a
    }

    /// Attempts to decode the artifact data according to some concrete type.
    ///
    /// The caller is responsible for checking [`ProcessorArtifactData::exec_version`]
    /// against the stage's current version first; decoding data written by a
    /// different version may succeed while producing a stale artifact.
    pub fn try_decode_artifact<A: ProcArtifact>(&self) -> Result<A, ProcError> {
        A::from_buf(self.artifact())
    }
}
