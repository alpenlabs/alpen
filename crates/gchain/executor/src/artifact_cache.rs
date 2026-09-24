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

    /// Discards the artifact one stage stored for a link.
    pub fn remove_artifact(&mut self, lref: &LinkRef<S>, proc_id: ProcId) {
        if let Some(atbl) = self.links.get_mut(lref) {
            atbl.remove(&proc_id);
            if atbl.is_empty() {
                self.links.remove(lref);
            }
        }
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
}

impl<S: GChainSpec> Default for ArtifactCache<S> {
    fn default() -> Self {
        Self::new()
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

    fn count(cache: &ArtifactCache<TestSpec>, lref: u8, id: &str) -> Option<u32> {
        cache
            .get_artifact::<CountArtifact>(&TestRef(lref), proc_id(id))
            .map(|a| a.0)
    }

    /// Two stages may produce artifacts of the same concrete type, so the stage
    /// that produced an artifact has to be what distinguishes them.
    #[test]
    fn test_get_artifact_downcasts_what_each_stage_inserted() {
        let mut cache = ArtifactCache::<TestSpec>::new();
        cache.insert_artifact(TestRef(1), proc_id("first"), Arc::new(CountArtifact(7)));
        cache.insert_artifact(TestRef(1), proc_id("second"), Arc::new(CountArtifact(9)));

        assert_eq!(count(&cache, 1, "first"), Some(7));
        assert_eq!(count(&cache, 1, "second"), Some(9));
        assert_eq!(count(&cache, 2, "first"), None);
        assert_eq!(count(&cache, 1, "absent"), None);
        assert_eq!(
            cache.get_artifact::<FlagArtifact>(&TestRef(1), proc_id("first")),
            None
        );

        cache.insert_artifact(TestRef(1), proc_id("first"), Arc::new(CountArtifact(8)));
        assert_eq!(count(&cache, 1, "first"), Some(8));
    }

    #[test]
    fn test_removals_drop_only_what_they_name() {
        let mut cache = ArtifactCache::<TestSpec>::new();
        for (lref, id) in [(1, "a"), (1, "b"), (2, "a"), (3, "a")] {
            cache.insert_artifact(
                TestRef(lref),
                proc_id(id),
                Arc::new(CountArtifact(u32::from(lref))),
            );
        }

        cache.remove_artifact(&TestRef(1), proc_id("b"));
        assert_eq!(count(&cache, 1, "a"), Some(1));
        assert_eq!(count(&cache, 1, "b"), None);

        cache.remove_link(&TestRef(2));
        assert_eq!(count(&cache, 2, "a"), None);

        cache.retain_links(&HashSet::from([TestRef(1)]));
        assert_eq!(count(&cache, 1, "a"), Some(1));
        assert_eq!(count(&cache, 3, "a"), None);
    }
}
