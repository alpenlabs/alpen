//! Minimal chain spec and processor stage for exercising the executor.

use std::sync::Arc;

use strata_gchain_types::*;

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct TestRef(pub u8);

impl GNodeRef for TestRef {}
impl GLinkRef for TestRef {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TestLink(pub u8);

impl GNode for TestLink {}
impl GLinkHeader for TestLink {}

impl GLink for TestLink {
    fn check_structurally_consistent(&self) -> bool {
        true
    }
}

pub(crate) struct TestSpec;

impl GChainSpec for TestSpec {
    type NodeRef = TestRef;
    type Node = TestLink;
    type LinkRef = TestRef;
    type LinkHeader = TestLink;
    type Link = TestLink;

    fn get_header_ref(lh: &TestLink) -> TestRef {
        TestRef(lh.0)
    }

    fn get_header_canonical_prev(_lh: &TestLink) -> Option<TestRef> {
        None
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct CountArtifact(pub u32);

impl ProcArtifact for CountArtifact {
    fn to_buf(&self) -> Result<Vec<u8>, ProcError> {
        Ok(self.0.to_be_bytes().to_vec())
    }

    fn from_buf(buf: &[u8]) -> Result<Self, ProcError> {
        let raw = buf
            .try_into()
            .map_err(|_| ProcError::Decode("bad CountArtifact length".into()))?;
        Ok(Self(u32::from_be_bytes(raw)))
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct FlagArtifact(pub bool);

impl ProcArtifact for FlagArtifact {
    fn to_buf(&self) -> Result<Vec<u8>, ProcError> {
        Ok(vec![self.0 as u8])
    }

    fn from_buf(buf: &[u8]) -> Result<Self, ProcError> {
        match buf {
            [b] => Ok(Self(*b != 0)),
            _ => Err(ProcError::Decode("bad FlagArtifact length".into())),
        }
    }

    fn is_link_valid(&self) -> bool {
        self.0
    }
}

/// Counts the links it's asked to process.
pub(crate) struct TestProc;

impl TestProc {
    pub(crate) const VERSION: u32 = 1;
}

impl GChainProc for TestProc {
    type Spec = TestSpec;
    type Artifact = CountArtifact;

    fn proc_version(&self) -> ProcVersion {
        Self::VERSION.into()
    }

    fn on_init(&self, _cur_node: &TestRef, _node: &TestLink) -> Result<(), ProcError> {
        Ok(())
    }

    fn process_link(
        &self,
        _lref: &TestRef,
        link: &TestLink,
        _ctx: &impl ProcContext<Self>,
    ) -> Result<CountArtifact, ProcError> {
        Ok(CountArtifact(link.0 as u32))
    }

    fn commit_outputs(
        &self,
        _path: &LinkPath<TestSpec>,
        _outputs: &[Arc<CountArtifact>],
    ) -> Result<(), ProcError> {
        Ok(())
    }

    fn uncommit_outputs(
        &self,
        _path: &LinkPath<TestSpec>,
        _outputs: &[Arc<CountArtifact>],
    ) -> Result<(), ProcError> {
        Ok(())
    }

    fn preprune_artifact(&self, _lref: &TestRef, _output: &CountArtifact) -> Result<(), ProcError> {
        Ok(())
    }

    fn prune_state_upto(&self, _nref: &TestRef) -> Result<(), ProcError> {
        Ok(())
    }
}
