//! Where the pipeline stands.

use std::collections::BTreeMap;
use std::fmt::{self, Debug, Display};

use strata_gchain_types::*;

use crate::errors::GExecError;

/// Position of a commit segment in the store's commit log.
///
/// Every commit is stored as the path it committed under the index after the
/// one before it, so the log can be walked back a segment at a time.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct CommitIndex(u64);

impl CommitIndex {
    /// The index the first commit into a fresh store goes under.
    pub fn first() -> Self {
        Self(0)
    }

    /// The index after this one.
    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }

    /// The index before this one, if there is one.
    pub fn prev(self) -> Option<Self> {
        self.0.checked_sub(1).map(Self)
    }
}

impl Display for CommitIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.0, f)
    }
}

impl From<u64> for CommitIndex {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl From<CommitIndex> for u64 {
    fn from(value: CommitIndex) -> Self {
        value.0
    }
}

/// Where a stage's aggregated state is, and where its history starts.
struct StagePosition<S: GChainSpec> {
    /// The last node the stage has processed and committed.
    committed_node: NodeRef<S>,

    /// The node the stage was initialized at.  It has nothing for the links
    /// committed before it.
    inited_at: NodeRef<S>,
}

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

    /// The oldest commit segment still covering part of the committed path.
    first_commit: CommitIndex,

    /// The index the next commit goes under.
    next_commit: CommitIndex,

    /// The position information for each processor stage.
    stages: BTreeMap<ProcId, StagePosition<S>>,
}

impl<S: GChainSpec> TrackingState<S> {
    /// Tracking for a pipeline sitting at a node with nothing committed past
    /// it and no stage initialized yet.
    pub fn new_at(base_node: NodeRef<S>) -> Self {
        Self {
            committed: LinkPath::new_at(base_node),
            first_commit: CommitIndex::first(),
            next_commit: CommitIndex::first(),
            stages: BTreeMap::new(),
        }
    }

    pub fn committed_node(&self) -> &NodeRef<S> {
        self.committed.terminal_node()
    }

    pub fn committed_path(&self) -> &LinkPath<S> {
        &self.committed
    }

    /// The oldest commit segment still covering part of the committed path,
    /// if there is one.
    pub fn first_commit(&self) -> CommitIndex {
        self.first_commit
    }

    /// The index the next commit goes under.
    pub fn next_commit(&self) -> CommitIndex {
        self.next_commit
    }

    /// The newest commit segment, if anything is committed.
    pub fn last_commit(&self) -> Option<CommitIndex> {
        self.next_commit
            .prev()
            .filter(|last| *last >= self.first_commit)
    }

    /// The node a processor stage has committed up to, if it has ever been
    /// initialized.
    pub fn get_stage_node(&self, proc_id: ProcId) -> Option<&NodeRef<S>> {
        self.stages.get(&proc_id).map(|pos| &pos.committed_node)
    }

    /// The node a processor stage was initialized at, if it ever has been.
    pub fn get_stage_floor(&self, proc_id: ProcId) -> Option<&NodeRef<S>> {
        self.stages.get(&proc_id).map(|pos| &pos.inited_at)
    }

    /// Records the node a stage has committed up to.  The first node recorded
    /// for a stage is the one it was initialized at.
    pub fn set_stage_node(&mut self, proc_id: ProcId, node: NodeRef<S>) {
        match self.stages.get_mut(&proc_id) {
            Some(pos) => pos.committed_node = node,
            None => {
                let pos = StagePosition {
                    committed_node: node.clone(),
                    inited_at: node,
                };
                self.stages.insert(proc_id, pos);
            }
        }
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
    /// node, as the commit stored under [`Self::next_commit`].
    pub fn extend_committed(&mut self, path: &LinkPath<S>) {
        let extended = self.committed.try_extend(path);
        debug_assert!(
            extended,
            "gchain: committed path continues from its terminal"
        );
        self.next_commit = self.next_commit.next();
    }

    /// Cuts the committed path back to a node on it, with the index the next
    /// commit goes under once the commit log is cut back to match.
    pub fn truncate_committed_to(
        &mut self,
        node: &NodeRef<S>,
        next_commit: CommitIndex,
    ) -> Result<(), GExecError> {
        let idx = self.get_committed_index_of(node)?;
        self.committed = self.committed.slice(0, idx);
        self.next_commit = next_commit;
        Ok(())
    }

    /// Moves the oldest node the pipeline can roll back to forward to a node
    /// on the committed path, giving up the links before it, with the oldest
    /// commit segment left once the commit log is cut to match.
    pub fn advance_base_to(
        &mut self,
        node: &NodeRef<S>,
        first_commit: CommitIndex,
    ) -> Result<(), GExecError> {
        let idx = self.get_committed_index_of(node)?;
        self.committed = self.committed.slice(idx, self.committed.len());
        self.first_commit = first_commit;
        Ok(())
    }

    fn get_committed_index_of(&self, node: &NodeRef<S>) -> Result<usize, GExecError> {
        self.committed
            .get_node_index(node)
            .ok_or_else(|| GExecError::node_not_on_committed_path(node))
    }
}

// Bounds fall on the ref types, not the marker spec type.
impl<S: GChainSpec> Clone for StagePosition<S> {
    fn clone(&self) -> Self {
        Self {
            committed_node: self.committed_node.clone(),
            inited_at: self.inited_at.clone(),
        }
    }
}

impl<S: GChainSpec> Debug for StagePosition<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StagePosition")
            .field("node", &self.committed_node)
            .field("floor", &self.inited_at)
            .finish()
    }
}

impl<S: GChainSpec> Clone for TrackingState<S> {
    fn clone(&self) -> Self {
        Self {
            committed: self.committed.clone(),
            first_commit: self.first_commit,
            next_commit: self.next_commit,
            stages: self.stages.clone(),
        }
    }
}

impl<S: GChainSpec> Debug for TrackingState<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TrackingState")
            .field("committed", &self.committed)
            .field("first_commit", &self.first_commit)
            .field("next_commit", &self.next_commit)
            .field("stages", &self.stages)
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
        assert_eq!(tracking.last_commit(), None);
        assert_eq!(
            tracking.get_stage_node(ProcId::from_str("a").expect("test: parse ProcId")),
            None
        );
    }

    #[test]
    fn test_stage_floor_stays_where_stage_was_first_put() {
        let proc_id = ProcId::from_str("a").expect("test: parse ProcId");
        let mut tracking = TrackingState::<TestSpec>::new_at(TestRef(1));

        tracking.set_stage_node(proc_id, TestRef(2));
        tracking.set_stage_node(proc_id, TestRef(3));

        assert_eq!(tracking.get_stage_node(proc_id), Some(&TestRef(3)));
        assert_eq!(tracking.get_stage_floor(proc_id), Some(&TestRef(2)));
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

        assert_eq!(tracking.last_commit(), Some(CommitIndex::from(0)));

        tracking
            .truncate_committed_to(&TestRef(2), CommitIndex::from(1))
            .expect("test: truncate");
        assert_eq!(*tracking.committed_path(), path(1, &[(10, 2)]));

        tracking.extend_committed(&suffix);
        assert_eq!(tracking.last_commit(), Some(CommitIndex::from(1)));
        tracking
            .advance_base_to(&TestRef(2), CommitIndex::from(1))
            .expect("test: advance base");
        assert_eq!(*tracking.committed_path(), path(2, &[(11, 3)]));
        assert_eq!(tracking.first_commit(), CommitIndex::from(1));
        let err = expect_err(
            tracking.truncate_committed_to(&TestRef(1), CommitIndex::from(1)),
            "node before the base to be refused",
        );
        assert!(matches!(err, GExecError::NodeNotOnCommittedPath(_)));
    }
}
