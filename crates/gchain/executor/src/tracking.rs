//! Where the pipeline stands.

use std::collections::BTreeMap;
use std::fmt::{self, Debug};

use strata_gchain_types::*;

use crate::errors::GExecError;

/// The committed path and how far each stage has committed along it.
///
/// This is everything an executor needs to pick up where it left off apart
/// from the artifacts themselves.  It's small (a handful of stages), so it's
/// persisted whole whenever any part of it changes and its parts can never be
/// seen to disagree.  The stages' nodes only differ from the path's terminal
/// while a commit is in progress; the executor brings them level on open.
pub struct TrackingState<S: GChainSpec> {
    /// The committed path, from the oldest node still rolled back to up to
    /// the committed node.
    committed: LinkPath<S>,

    /// The last nodes that each processor stage has processed and committed.
    stage_nodes: BTreeMap<ProcId, NodeRef<S>>,
}

impl<S: GChainSpec> TrackingState<S> {
    /// Tracking for a pipeline sitting at a node with nothing committed past
    /// it and no stage initialized yet.
    pub fn new_at(base_node: NodeRef<S>) -> Self {
        Self {
            committed: LinkPath::new_at(base_node),
            stage_nodes: BTreeMap::new(),
        }
    }

    pub fn committed_node(&self) -> &NodeRef<S> {
        self.committed.terminal_node()
    }

    pub fn committed_path(&self) -> &LinkPath<S> {
        &self.committed
    }

    /// The node a processor stage has committed up to, if it has ever been
    /// initialized.
    pub fn get_stage_node(&self, proc_id: ProcId) -> Option<&NodeRef<S>> {
        self.stage_nodes.get(&proc_id)
    }

    pub fn set_stage_node(&mut self, proc_id: ProcId, node: NodeRef<S>) {
        self.stage_nodes.insert(proc_id, node);
    }

    pub fn is_committed(&self, lref: &LinkRef<S>) -> bool {
        self.committed.links().contains(lref)
    }

    /// The committed links from a node on the committed path up to the
    /// committed node.
    pub fn committed_path_from(&self, node: &NodeRef<S>) -> Result<LinkPath<S>, GExecError> {
        let idx = self.get_committed_index_of(node)?;
        Ok(self.committed.slice(idx, self.committed.len()))
    }

    /// Extends the committed path by a path continuing from the committed
    /// node.
    pub fn extend_committed(&mut self, path: &LinkPath<S>) {
        let extended = self.committed.try_extend(path);
        debug_assert!(
            extended,
            "gchain: committed path continues from its terminal"
        );
    }

    /// Cuts the committed path back to a node on it.
    pub fn truncate_committed_to(&mut self, node: &NodeRef<S>) -> Result<(), GExecError> {
        let idx = self.get_committed_index_of(node)?;
        self.committed = self.committed.slice(0, idx);
        Ok(())
    }

    /// Moves the oldest node the pipeline can roll back to forward to a node
    /// on the committed path, giving up the links before it.
    pub fn advance_base_to(&mut self, node: &NodeRef<S>) -> Result<(), GExecError> {
        let idx = self.get_committed_index_of(node)?;
        self.committed = self.committed.slice(idx, self.committed.len());
        Ok(())
    }

    fn get_committed_index_of(&self, node: &NodeRef<S>) -> Result<usize, GExecError> {
        self.committed
            .get_node_index(node)
            .ok_or_else(|| GExecError::NodeNotOnCommittedPath(format!("{node:?}")))
    }
}

// Bounds fall on the ref types, not the marker spec type.
impl<S: GChainSpec> Clone for TrackingState<S> {
    fn clone(&self) -> Self {
        Self {
            committed: self.committed.clone(),
            stage_nodes: self.stage_nodes.clone(),
        }
    }
}

impl<S: GChainSpec> Debug for TrackingState<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TrackingState")
            .field("committed", &self.committed)
            .field("stage_nodes", &self.stage_nodes)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;
    use crate::test_support::*;

    /// A path from a base node through links given as `(lref, target)`.
    fn path(base: u8, steps: &[(u8, u8)]) -> LinkPath<TestSpec> {
        LinkPath::from_steps(
            TestRef(base),
            steps.iter().map(|(l, n)| (TestRef(*l), TestRef(*n))),
        )
    }

    #[test]
    fn test_new_tracking_is_committed_at_genesis_with_no_stages() {
        let tracking = TrackingState::<TestSpec>::new_at(TestRef(1));

        assert_eq!(tracking.committed_node(), &TestRef(1));
        assert!(tracking.committed_path().is_empty());
        assert_eq!(
            tracking.get_stage_node(ProcId::from_str("a").expect("test: parse ProcId")),
            None
        );
    }

    #[test]
    fn test_committed_path_edits() {
        let mut tracking = TrackingState::<TestSpec>::new_at(TestRef(1));

        tracking.extend_committed(&path(1, &[(10, 2), (11, 3)]));
        assert_eq!(tracking.committed_node(), &TestRef(3));
        assert!(tracking.is_committed(&TestRef(11)));
        assert!(!tracking.is_committed(&TestRef(20)));

        let suffix = tracking
            .committed_path_from(&TestRef(2))
            .expect("test: suffix");
        assert_eq!(suffix, path(2, &[(11, 3)]));
        let err = expect_err(
            tracking.committed_path_from(&TestRef(9)),
            "node off the path to be refused",
        );
        assert!(matches!(err, GExecError::NodeNotOnCommittedPath(_)));

        tracking
            .truncate_committed_to(&TestRef(2))
            .expect("test: truncate");
        assert_eq!(*tracking.committed_path(), path(1, &[(10, 2)]));

        tracking.extend_committed(&suffix);
        tracking
            .advance_base_to(&TestRef(2))
            .expect("test: advance base");
        assert_eq!(*tracking.committed_path(), path(2, &[(11, 3)]));
        let err = expect_err(
            tracking.truncate_committed_to(&TestRef(1)),
            "node before the base to be refused",
        );
        assert!(matches!(err, GExecError::NodeNotOnCommittedPath(_)));
    }
}
