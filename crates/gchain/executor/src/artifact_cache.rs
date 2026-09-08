use std::collections::*;
use std::sync::Arc;

use strata_gchain_types::*;

/// Cached artifacts from links that we've extracted and determined might be
/// useful for later proc stages.
///
/// Artifacts are keyed by the processor stage that produced them, since that's
/// how processor stages name their dependencies (see [`ProcDeps`]).  Multiple
/// stages may produce artifacts of the same concrete type.
pub struct ArtifactCache<S: GChainSpec> {
    links: HashMap<LinkRef<S>, BTreeMap<ProcId, Arc<dyn DynProcArtifact>>>,
}

impl<S: GChainSpec> ArtifactCache<S> {
    /// Creates a new empty cache.
    pub fn new() -> Self {
        Self {
            links: HashMap::new(),
        }
    }

    /// Stores the artifact a processor stage produced for a link, replacing any
    /// artifact that stage had already stored for it.
    pub fn insert_artifact(
        &mut self,
        lref: LinkRef<S>,
        proc_id: ProcId,
        artifact: Arc<dyn DynProcArtifact>,
    ) {
        self.links
            .entry(lref)
            .or_default()
            .insert(proc_id, artifact);
    }

    /// Gets the type-erased artifact some processor stage stored for a link.
    pub fn get_artifact_dyn(
        &self,
        lref: &LinkRef<S>,
        proc_id: ProcId,
    ) -> Option<&Arc<dyn DynProcArtifact>> {
        self.links.get(lref).and_then(|atbl| atbl.get(&proc_id))
    }

    /// Gets the artifact some processor stage stored for a link, downcast to its
    /// concrete type.
    ///
    /// Returns `None` if the stage stored no artifact for the link, or if the
    /// artifact it stored isn't of type `A`.
    pub fn get_artifact<A: ProcArtifact>(
        &self,
        lref: &LinkRef<S>,
        proc_id: ProcId,
    ) -> Option<Arc<A>> {
        let artifact = self.get_artifact_dyn(lref, proc_id)?;
        Arc::clone(artifact).into_any_arc().downcast::<A>().ok()
    }

    /// Discards every artifact stored for a link.
    pub fn remove_link(&mut self, lref: &LinkRef<S>) {
        self.links.remove(lref);
    }

    /// Discards the artifacts for every link outside the provided set, such as
    /// after committing or abandoning a path.
    pub fn retain_links(&mut self, keep: &HashSet<LinkRef<S>>) {
        self.links.retain(|lref, _| keep.contains(lref));
    }

    /// The number of links with artifacts currently cached.
    pub fn cached_link_count(&self) -> usize {
        self.links.len()
    }
}

impl<S: GChainSpec> Default for ArtifactCache<S> {
    fn default() -> Self {
        Self::new()
    }
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

    /// Drops the oldest `count` steps, rebasing the history onto the node they
    /// ended at.  Used once those steps are committed and can't be rolled back.
    pub fn drain_committed(&mut self, count: usize, new_base: NodeRef<P::Spec>) {
        let count = count.min(self.steps.len());
        self.steps.drain(..count);
        self.base = new_base;
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;
    use crate::test_support::*;

    fn proc_id(s: &str) -> ProcId {
        ProcId::from_str(s).expect("test: parse ProcId")
    }

    #[test]
    fn test_get_artifact_returns_stored_artifact() {
        let lref = TestRef(1);
        let mut cache = ArtifactCache::<TestSpec>::new();
        cache.insert_artifact(lref, proc_id("count"), Arc::new(CountArtifact(7)));

        let fetched = cache
            .get_artifact::<CountArtifact>(&lref, proc_id("count"))
            .expect("test: fetch artifact");
        assert_eq!(*fetched, CountArtifact(7));
    }

    /// Two stages may produce artifacts of the same concrete type, so the stage
    /// that produced an artifact has to be what distinguishes them.
    #[test]
    fn test_get_artifact_separates_procs_sharing_artifact_type() {
        let lref = TestRef(1);
        let mut cache = ArtifactCache::<TestSpec>::new();
        cache.insert_artifact(lref, proc_id("first"), Arc::new(CountArtifact(7)));
        cache.insert_artifact(lref, proc_id("second"), Arc::new(CountArtifact(9)));

        let first = cache
            .get_artifact::<CountArtifact>(&lref, proc_id("first"))
            .expect("test: fetch first artifact");
        let second = cache
            .get_artifact::<CountArtifact>(&lref, proc_id("second"))
            .expect("test: fetch second artifact");
        assert_eq!(*first, CountArtifact(7));
        assert_eq!(*second, CountArtifact(9));
    }

    #[test]
    fn test_get_artifact_with_mismatched_type_returns_none() {
        let lref = TestRef(1);
        let mut cache = ArtifactCache::<TestSpec>::new();
        cache.insert_artifact(lref, proc_id("count"), Arc::new(CountArtifact(7)));

        assert_eq!(
            cache.get_artifact::<FlagArtifact>(&lref, proc_id("count")),
            None
        );
    }

    #[test]
    fn test_get_artifact_for_absent_link_or_proc_returns_none() {
        let lref = TestRef(1);
        let mut cache = ArtifactCache::<TestSpec>::new();
        cache.insert_artifact(lref, proc_id("count"), Arc::new(CountArtifact(7)));

        assert_eq!(
            cache.get_artifact::<CountArtifact>(&TestRef(2), proc_id("count")),
            None
        );
        assert_eq!(
            cache.get_artifact::<CountArtifact>(&lref, proc_id("absent")),
            None
        );
    }

    /// The executor checks link validity without knowing the concrete artifact
    /// type, so it has to work through the erased view.
    #[test]
    fn test_is_link_valid_visible_through_erasure() {
        let lref = TestRef(1);
        let mut cache = ArtifactCache::<TestSpec>::new();
        cache.insert_artifact(lref, proc_id("valid"), Arc::new(FlagArtifact(true)));
        cache.insert_artifact(lref, proc_id("invalid"), Arc::new(FlagArtifact(false)));
        cache.insert_artifact(lref, proc_id("count"), Arc::new(CountArtifact(7)));

        let is_valid = |id: &str| {
            cache
                .get_artifact_dyn(&lref, proc_id(id))
                .expect("test: fetch artifact")
                .is_link_valid()
        };
        assert!(is_valid("valid"));
        assert!(!is_valid("invalid"));
        // Stages not involved in validation get the default.
        assert!(is_valid("count"));
    }

    #[test]
    fn test_insert_artifact_replaces_previous_artifact() {
        let lref = TestRef(1);
        let mut cache = ArtifactCache::<TestSpec>::new();
        cache.insert_artifact(lref, proc_id("count"), Arc::new(CountArtifact(7)));
        cache.insert_artifact(lref, proc_id("count"), Arc::new(CountArtifact(9)));

        let fetched = cache
            .get_artifact::<CountArtifact>(&lref, proc_id("count"))
            .expect("test: fetch artifact");
        assert_eq!(*fetched, CountArtifact(9));
    }

    #[test]
    fn test_remove_link_discards_every_artifact_for_it() {
        let kept = TestRef(1);
        let removed = TestRef(2);
        let mut cache = ArtifactCache::<TestSpec>::new();
        cache.insert_artifact(kept, proc_id("count"), Arc::new(CountArtifact(7)));
        cache.insert_artifact(removed, proc_id("count"), Arc::new(CountArtifact(8)));
        cache.insert_artifact(removed, proc_id("flag"), Arc::new(FlagArtifact(true)));

        cache.remove_link(&removed);

        assert_eq!(
            cache.get_artifact::<CountArtifact>(&removed, proc_id("count")),
            None
        );
        assert_eq!(
            cache.get_artifact::<FlagArtifact>(&removed, proc_id("flag")),
            None
        );
        assert!(
            cache
                .get_artifact::<CountArtifact>(&kept, proc_id("count"))
                .is_some()
        );
    }

    #[test]
    fn test_retain_links_drops_everything_else() {
        let kept = TestRef(1);
        let mut cache = ArtifactCache::<TestSpec>::new();
        cache.insert_artifact(kept, proc_id("count"), Arc::new(CountArtifact(7)));
        cache.insert_artifact(TestRef(2), proc_id("count"), Arc::new(CountArtifact(8)));
        cache.insert_artifact(TestRef(3), proc_id("count"), Arc::new(CountArtifact(9)));
        assert_eq!(cache.cached_link_count(), 3);

        cache.retain_links(&HashSet::from([kept]));

        assert_eq!(cache.cached_link_count(), 1);
        assert!(
            cache
                .get_artifact::<CountArtifact>(&kept, proc_id("count"))
                .is_some()
        );
    }

    /// The executor persists artifacts on behalf of stages, so what a stage
    /// encodes has to come back as the same artifact.
    #[test]
    fn test_artifact_roundtrips_through_stored_data() {
        let version = ProcVersion::from(TestProc::VERSION);
        let data = ProcessorArtifactData::from_artifact(version, &CountArtifact(7))
            .expect("test: encode artifact");

        assert_eq!(data.exec_version(), version);
        assert_eq!(
            data.try_decode_artifact::<CountArtifact>()
                .expect("test: decode artifact"),
            CountArtifact(7)
        );
    }

    /// Artifacts are stored type-erased, so the executor encodes them without
    /// knowing which stage produced them.
    #[test]
    fn test_artifact_encodes_through_erasure() {
        let artifact: Arc<dyn DynProcArtifact> = Arc::new(CountArtifact(7));

        let buf = artifact.to_buf_dyn().expect("test: encode artifact");

        assert_eq!(
            CountArtifact::from_buf(&buf).expect("test: decode artifact"),
            CountArtifact(7)
        );
    }
}
