//! Minimal chain spec, provider, and processor stages for exercising the
//! executor.

use std::collections::{HashMap, HashSet};
use std::mem;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use strata_gchain_types::*;

use crate::config::{PipelineBuilder, StagePipeline};
use crate::mem_store::MemExecutorStore;
use crate::store::ExecutorStore;

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct TestRef(pub u8);

impl GNodeRef for TestRef {}
impl GLinkRef for TestRef {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TestLink(pub u8);

impl GLinkHeader for TestLink {}

impl GLink for TestLink {
    fn check_structurally_consistent(&self) -> bool {
        true
    }
}

pub(crate) struct TestSpec;

impl GChainSpec for TestSpec {
    type NodeRef = TestRef;
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

/// Chain provider over links added by hand.  A link's body is just its ref.
pub(crate) struct TestProvider {
    links: Mutex<HashMap<TestRef, LinkEndpoints<TestSpec>>>,
}

impl TestProvider {
    pub(crate) fn new() -> Self {
        Self {
            links: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn add_link(&self, lref: u8, origin: u8, target: u8) {
        self.links.lock().unwrap().insert(
            TestRef(lref),
            LinkEndpoints::new(TestRef(origin), TestRef(target)),
        );
    }

    fn links_where(&self, pred: impl Fn(&LinkEndpoints<TestSpec>) -> bool) -> Vec<TestRef> {
        let mut found: Vec<_> = self
            .links
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, e)| pred(e))
            .map(|(l, _)| *l)
            .collect();
        found.sort();
        found
    }
}

impl ChainProvider for TestProvider {
    type Spec = TestSpec;

    fn fetch_link_header(&self, lref: &TestRef) -> Result<Option<TestLink>, ProviderError> {
        self.fetch_link(lref)
    }

    fn fetch_link(&self, lref: &TestRef) -> Result<Option<TestLink>, ProviderError> {
        let known = self.links.lock().unwrap().contains_key(lref);
        Ok(known.then_some(TestLink(lref.0)))
    }

    fn fetch_link_endpoints(
        &self,
        lref: &TestRef,
    ) -> Result<Option<LinkEndpoints<TestSpec>>, ProviderError> {
        Ok(self.links.lock().unwrap().get(lref).cloned())
    }

    fn fetch_forward_links(&self, nref: &TestRef) -> Result<Vec<TestRef>, ProviderError> {
        Ok(self.links_where(|e| e.origin() == nref))
    }

    fn fetch_backward_links(&self, nref: &TestRef) -> Result<Vec<TestRef>, ProviderError> {
        Ok(self.links_where(|e| e.target() == nref))
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

/// What the executor asked a [`TestProc`] to do, in order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ProcEvent {
    Init(TestRef),
    Process(TestRef),
    Commit(Vec<TestRef>),
    Uncommit(Vec<TestRef>),
    Preprune(TestRef),
    Prune(TestRef),
}

pub(crate) type EventLog = Arc<Mutex<Vec<ProcEvent>>>;

/// Records every call the executor makes and rejects the links it's told to.
pub(crate) struct TestProc {
    version: u32,
    reject: HashSet<TestRef>,
    events: EventLog,
}

impl TestProc {
    pub(crate) fn new() -> Self {
        Self {
            version: 1,
            reject: HashSet::new(),
            events: EventLog::default(),
        }
    }

    pub(crate) fn with_version(mut self, version: u32) -> Self {
        self.version = version;
        self
    }

    pub(crate) fn rejecting(mut self, lrefs: impl IntoIterator<Item = u8>) -> Self {
        self.reject = lrefs.into_iter().map(TestRef).collect();
        self
    }

    /// A handle onto the event log that outlives handing the stage to a
    /// pipeline.
    pub(crate) fn events(&self) -> EventLog {
        Arc::clone(&self.events)
    }

    fn record(&self, event: ProcEvent) {
        self.events.lock().unwrap().push(event);
    }
}

/// A pipeline of the given stages under the IDs "a", "b", ... in order, with
/// no deps between them.
pub(crate) fn pipeline_of(procs: Vec<TestProc>) -> StagePipeline<TestSpec> {
    let mut builder = PipelineBuilder::new();
    for (idx, proc) in procs.into_iter().enumerate() {
        let name = char::from(b'a' + idx as u8).to_string();
        let proc_id = ProcId::from_str(&name).expect("test: parse ProcId");
        builder = builder
            .add_stage(proc_id, proc, ProcDeps::new(Vec::new(), Vec::new()))
            .expect("test: add stage");
    }
    builder.build()
}

/// The artifact a stage has stored for a link.
pub(crate) fn stored_artifact(
    store: &MemExecutorStore<TestSpec>,
    lref: TestRef,
    proc_id: ProcId,
) -> Option<ProcessorArtifactData> {
    store
        .load_link_artifacts(&lref)
        .expect("test: load artifacts")
        .into_iter()
        .find(|record| record.proc_id() == proc_id)
        .map(|record| record.into_parts().2)
}

/// Whether the store holds any artifact for a link.
pub(crate) fn has_stored(store: &MemExecutorStore<TestSpec>, lref: u8) -> bool {
    store
        .has_link_artifacts(&TestRef(lref))
        .expect("test: check artifacts")
}

/// Drains the events recorded so far.
pub(crate) fn take_events(log: &EventLog) -> Vec<ProcEvent> {
    mem::take(&mut *log.lock().unwrap())
}

/// Unwraps the error of a result whose success type may not be `Debug`.
pub(crate) fn expect_err<T, E>(res: Result<T, E>, what: &str) -> E {
    match res {
        Ok(_) => panic!("test: expected {what}"),
        Err(err) => err,
    }
}

impl GChainProc for TestProc {
    type Spec = TestSpec;
    type Artifact = FlagArtifact;

    fn proc_version(&self) -> ProcVersion {
        self.version.into()
    }

    fn on_init(&self, cur_node: &TestRef) -> Result<(), ProcError> {
        self.record(ProcEvent::Init(*cur_node));
        Ok(())
    }

    fn process_link(
        &self,
        lref: &TestRef,
        _link: &TestLink,
        _ctx: &impl ProcContext<Self>,
    ) -> Result<FlagArtifact, ProcError> {
        self.record(ProcEvent::Process(*lref));
        Ok(FlagArtifact(!self.reject.contains(lref)))
    }

    fn commit_outputs(
        &self,
        path: &LinkPath<TestSpec>,
        _outputs: &[Arc<FlagArtifact>],
    ) -> Result<(), ProcError> {
        self.record(ProcEvent::Commit(path.links().to_vec()));
        Ok(())
    }

    fn uncommit_outputs(
        &self,
        path: &LinkPath<TestSpec>,
        _outputs: &[Arc<FlagArtifact>],
    ) -> Result<(), ProcError> {
        self.record(ProcEvent::Uncommit(path.links().to_vec()));
        Ok(())
    }

    fn preprune_artifact(&self, lref: &TestRef, _output: &FlagArtifact) -> Result<(), ProcError> {
        self.record(ProcEvent::Preprune(*lref));
        Ok(())
    }

    fn prune_state_upto(&self, nref: &TestRef) -> Result<(), ProcError> {
        self.record(ProcEvent::Prune(*nref));
        Ok(())
    }
}
