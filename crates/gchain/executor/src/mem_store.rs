//! In-memory executor store.

use std::collections::BTreeMap;
use std::sync::Mutex;

use strata_gchain_types::*;

use crate::store::{ExecutorStore, LinkRecord};

/// In-memory [`ExecutorStore`], for tests and executors that don't need to
/// survive a restart.
pub struct MemExecutorStore<S: GChainSpec> {
    inner: Mutex<MemStoreInner<S>>,
}

struct MemStoreInner<S: GChainSpec> {
    artifacts: BTreeMap<(LinkRef<S>, ProcId), ProcessorArtifactData>,
    links: BTreeMap<LinkRef<S>, LinkEndpoints<S>>,
    committed_nodes: BTreeMap<ProcId, NodeRef<S>>,
    committed_path: Option<PathDesc<S>>,
}

impl<S: GChainSpec> MemExecutorStore<S> {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(MemStoreInner {
                artifacts: BTreeMap::new(),
                links: BTreeMap::new(),
                committed_nodes: BTreeMap::new(),
                committed_path: None,
            }),
        }
    }

    fn with_inner<R>(&self, f: impl FnOnce(&mut MemStoreInner<S>) -> R) -> Result<R, BoxedError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| "executor store mutex poisoned")?;
        Ok(f(&mut inner))
    }
}

impl<S: GChainSpec> Default for MemExecutorStore<S> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S: GChainSpec> ExecutorStore for MemExecutorStore<S> {
    type Spec = S;

    fn store_artifact(
        &self,
        lref: &LinkRef<S>,
        proc_id: ProcId,
        data: &ProcessorArtifactData,
    ) -> Result<(), BoxedError> {
        self.with_inner(|inner| {
            inner
                .artifacts
                .insert((lref.clone(), proc_id), data.clone());
        })
    }

    fn load_artifact(
        &self,
        lref: &LinkRef<S>,
        proc_id: ProcId,
    ) -> Result<Option<ProcessorArtifactData>, BoxedError> {
        self.with_inner(|inner| inner.artifacts.get(&(lref.clone(), proc_id)).cloned())
    }

    fn discard_link_artifacts(&self, lref: &LinkRef<S>) -> Result<(), BoxedError> {
        self.with_inner(|inner| inner.artifacts.retain(|(l, _), _| l != lref))
    }

    fn store_link(&self, record: &LinkRecord<S>) -> Result<(), BoxedError> {
        self.with_inner(|inner| {
            inner
                .links
                .insert(record.lref().clone(), record.endpoints().clone());
        })
    }

    fn discard_link(&self, lref: &LinkRef<S>) -> Result<(), BoxedError> {
        self.with_inner(|inner| {
            inner.links.remove(lref);
        })
    }

    fn load_links(&self) -> Result<Vec<LinkRecord<S>>, BoxedError> {
        self.with_inner(|inner| {
            inner
                .links
                .iter()
                .map(|(l, e)| LinkRecord::new(l.clone(), e.clone()))
                .collect()
        })
    }

    fn store_committed_node(&self, proc_id: ProcId, node: &NodeRef<S>) -> Result<(), BoxedError> {
        self.with_inner(|inner| {
            inner.committed_nodes.insert(proc_id, node.clone());
        })
    }

    fn load_committed_node(&self, proc_id: ProcId) -> Result<Option<NodeRef<S>>, BoxedError> {
        self.with_inner(|inner| inner.committed_nodes.get(&proc_id).cloned())
    }

    fn store_committed_path(&self, path: &PathDesc<S>) -> Result<(), BoxedError> {
        self.with_inner(|inner| inner.committed_path = Some(path.clone()))
    }

    fn load_committed_path(&self) -> Result<Option<PathDesc<S>>, BoxedError> {
        self.with_inner(|inner| inner.committed_path.clone())
    }
}
