//! Storage-backed ancestry queries for an explicitly selected OL block branch.

use std::ops::ControlFlow;
use std::sync::Arc;

use strata_db_types::{DbError, DbResult};
use strata_identifiers::{EpochCommitment, OLBlockCommitment, OLBlockId};
use strata_ol_chain_types_v1::OLBlockHeaderV1;
use strata_storage::OLBlockManager;
use tokio::task::spawn_blocking;

use crate::BlockAncestryError;

/// Resolves ancestry on the supplied block's branch, independently of fork choice.
#[derive(Clone)]
#[expect(
    missing_debug_implementations,
    reason = "storage managers do not implement Debug"
)]
pub struct BlockAncestryProvider {
    ol_block: Arc<OLBlockManager>,
}

/// Blocks from the beginning of an epoch through the selected block, in execution order,
/// together with the preceding epoch.
#[derive(Debug, PartialEq, Eq)]
pub struct EpochBlockAncestry {
    previous_epoch: EpochCommitment,
    blocks: Vec<OLBlockCommitment>,
}

impl EpochBlockAncestry {
    /// Assembles an expected ancestry from parts recorded by a test history, without walking
    /// storage.
    ///
    /// `blocks` must be in execution order and end at the selected block; only nonemptiness is
    /// checked.
    ///
    /// # Panics
    ///
    /// Panics when `blocks` is empty.
    #[cfg(any(test, feature = "test-utils"))]
    pub(crate) fn from_parts(
        previous_epoch: EpochCommitment,
        blocks: Vec<OLBlockCommitment>,
    ) -> Self {
        assert!(!blocks.is_empty(), "epoch ancestry must contain a block");
        Self {
            previous_epoch,
            blocks,
        }
    }

    /// Returns the preceding epoch resolved from the selected block's ancestry.
    pub fn previous_epoch(&self) -> EpochCommitment {
        self.previous_epoch
    }

    /// Returns the nonempty sequence through the starting block, excluding the previous terminal.
    pub fn blocks(&self) -> &[OLBlockCommitment] {
        &self.blocks
    }

    /// Requires the preceding epoch to end at `expected`.
    pub fn require_previous_terminal(
        &self,
        expected: OLBlockCommitment,
    ) -> Result<(), BlockAncestryError> {
        let actual = self.previous_epoch.to_block_commitment();
        if actual != expected {
            return Err(BlockAncestryError::PreviousTerminalMismatch { expected, actual });
        }
        Ok(())
    }

    /// Takes the preceding epoch and the blocks in execution order.
    pub fn into_parts(self) -> (EpochCommitment, Vec<OLBlockCommitment>) {
        (self.previous_epoch, self.blocks)
    }
}

impl BlockAncestryProvider {
    /// Creates a provider backed by the block manager.
    pub fn new(ol_block: Arc<OLBlockManager>) -> Self {
        Self { ol_block }
    }

    /// Finds the preceding epoch along `start`'s parent chain.
    ///
    /// Excludes `start`, even when terminal. Epoch zero has no previous epoch.
    /// Missing headers or nonconsecutive slot/epoch boundaries return an error.
    /// A retained terminal header suffices; its body and ancestors are not required.
    /// Takes `start` as given and checks only its ancestors against their parent links.
    pub fn find_previous_epoch_commitment_blocking(
        &self,
        start: &OLBlockHeaderV1,
    ) -> Result<EpochCommitment, BlockAncestryError> {
        find_previous_epoch_commitment(start, |id| self.ol_block.get_ol_header_blocking(id), |_| {})
    }

    /// Async counterpart of [`Self::find_previous_epoch_commitment_blocking`].
    pub async fn find_previous_epoch_commitment_async(
        &self,
        start: &OLBlockHeaderV1,
    ) -> Result<EpochCommitment, BlockAncestryError> {
        let provider = self.clone();
        let start = start.clone();
        spawn_blocking(move || provider.find_previous_epoch_commitment_blocking(&start))
            .await
            .map_err(DbError::from)?
    }

    /// Collects current-epoch commitments through `start` in execution order.
    ///
    /// Includes `start` and excludes the previous terminal, whose epoch commitment is returned
    /// separately.
    /// Uses the boundary and failure rules of
    /// [`Self::find_previous_epoch_commitment_blocking`]. Takes `start` as given and checks
    /// only its ancestors against their parent links.
    pub fn collect_epoch_block_commitments_blocking(
        &self,
        start: &OLBlockHeaderV1,
    ) -> Result<EpochBlockAncestry, BlockAncestryError> {
        let mut blocks = vec![start.compute_block_commitment()];
        let previous_epoch = find_previous_epoch_commitment(
            start,
            |id| self.ol_block.get_ol_header_blocking(id),
            |block| blocks.push(block),
        )?;
        blocks.reverse();
        Ok(EpochBlockAncestry {
            previous_epoch,
            blocks,
        })
    }

    /// Async counterpart of [`Self::collect_epoch_block_commitments_blocking`].
    pub async fn collect_epoch_block_commitments_async(
        &self,
        start: &OLBlockHeaderV1,
    ) -> Result<EpochBlockAncestry, BlockAncestryError> {
        let provider = self.clone();
        let start = start.clone();
        spawn_blocking(move || provider.collect_epoch_block_commitments_blocking(&start))
            .await
            .map_err(DbError::from)?
    }

    /// Loads and validates `epoch`'s terminal before collecting its block commitments.
    pub fn collect_epoch_block_commitments_at_blocking(
        &self,
        epoch: EpochCommitment,
    ) -> Result<EpochBlockAncestry, BlockAncestryError> {
        let terminal = self
            .ol_block
            .get_ol_header_blocking(*epoch.last_blkid())?
            .ok_or(BlockAncestryError::MissingEpochTerminal(epoch))?;
        let actual =
            EpochCommitment::from_terminal(terminal.epoch(), terminal.compute_block_commitment());
        if actual != epoch {
            return Err(BlockAncestryError::EpochTerminalMismatch {
                expected: epoch,
                actual,
            });
        }
        if !terminal.is_terminal() {
            return Err(BlockAncestryError::MissingTerminalFlag(epoch));
        }
        self.collect_epoch_block_commitments_blocking(&terminal)
    }

    /// Async counterpart of [`Self::collect_epoch_block_commitments_at_blocking`].
    pub async fn collect_epoch_block_commitments_at_async(
        &self,
        epoch: EpochCommitment,
    ) -> Result<EpochBlockAncestry, BlockAncestryError> {
        let provider = self.clone();
        spawn_blocking(move || provider.collect_epoch_block_commitments_at_blocking(epoch))
            .await
            .map_err(DbError::from)?
    }

    /// Collects up to `max_blocks` commitments by following parents from `start`,
    /// returning them in ascending slot order.
    ///
    /// Includes `start` and crosses epoch boundaries. Stops at the limit, genesis
    /// (included), or the stored history base (excluded). Returns an empty list
    /// when the limit is zero or `start` is at or below the history base.
    ///
    /// Missing or inconsistent ancestry and storage failures return an error,
    /// never a partial list. Takes `start` as given and checks only its ancestors against their
    /// parent links.
    pub fn collect_block_commitments_blocking(
        &self,
        start: &OLBlockHeaderV1,
        max_blocks: usize,
    ) -> Result<Vec<OLBlockCommitment>, BlockAncestryError> {
        // A zero limit needs no storage reads, including the history-base lookup.
        if max_blocks == 0 {
            return Ok(Vec::new());
        }
        // Read the history base once for this walk. Its retained header does not imply
        // a retained body, so collection excludes the history base itself.
        let history_base = self
            .ol_block
            .get_history_base_blocking()?
            .map(|base| base.to_block_commitment());
        collect_block_commitments(start, max_blocks, history_base, |id| {
            self.ol_block.get_ol_header_blocking(id)
        })
    }

    /// Async counterpart of [`Self::collect_block_commitments_blocking`].
    pub async fn collect_block_commitments_async(
        &self,
        start: &OLBlockHeaderV1,
        max_blocks: usize,
    ) -> Result<Vec<OLBlockCommitment>, BlockAncestryError> {
        let provider = self.clone();
        let start = start.clone();
        spawn_blocking(move || provider.collect_block_commitments_blocking(&start, max_blocks))
            .await
            .map_err(DbError::from)?
    }
}

// Core logic for the bounded walk. It returns selected commitments in ascending slot order,
// stopping at the count, after including genesis, or immediately before the history base.
// A zero limit or a start at or below the base returns an empty list.
// Reaching the base requires both an exact-slot parent link and the history base's block ID.
fn collect_block_commitments(
    start: &OLBlockHeaderV1,
    max_blocks: usize,
    history_base: Option<OLBlockCommitment>,
    fetch: impl FnMut(OLBlockId) -> DbResult<Option<OLBlockHeaderV1>>,
) -> Result<Vec<OLBlockCommitment>, BlockAncestryError> {
    let mut blocks = Vec::new();
    if max_blocks == 0 || history_base.is_some_and(|base| start.slot() <= base.slot()) {
        return Ok(blocks);
    }
    walk_ancestry(start, fetch, |header, block| {
        // The early return and adjacent-slot checks below stop before reaching the base.
        debug_assert!(history_base.is_none_or(|base| block.slot() > base.slot()));
        blocks.push(block);
        if blocks.len() == max_blocks || header.slot() == 0 {
            return Ok(ControlFlow::Break(()));
        }
        if let Some(base) = history_base {
            if header.parent_blkid() == base.blkid() {
                if header.slot().checked_sub(1) != Some(base.slot()) {
                    return Err(BlockAncestryError::ParentSlotMismatch {
                        child: block,
                        parent_slot: base.slot(),
                    });
                }
                return Ok(ControlFlow::Break(()));
            }
            // At the adjacent slot, only the history base can end the walk. Check its ID
            // without reading the history-base header.
            if header.slot().checked_sub(1) == Some(base.slot()) {
                return Err(BlockAncestryError::UnexpectedAncestor {
                    expected: *base.blkid(),
                    actual: *header.parent_blkid(),
                });
            }
        }
        Ok(ControlFlow::Continue(()))
    })?;
    blocks.reverse();
    Ok(blocks)
}

// Returns the preceding epoch ending at the first terminal ancestor, walking newest-first from
// `start`. Excludes `start` and the terminal ancestor from `visit`, which receives intervening
// nonterminal ancestors. An epoch-zero start fails. Nonterminal ancestors must remain in
// the start epoch, and the terminal must be in the immediately preceding epoch. The supplied
// start header is trusted by this core.
fn find_previous_epoch_commitment(
    start: &OLBlockHeaderV1,
    fetch: impl FnMut(OLBlockId) -> DbResult<Option<OLBlockHeaderV1>>,
    mut visit: impl FnMut(OLBlockCommitment),
) -> Result<EpochCommitment, BlockAncestryError> {
    let previous_epoch = start.epoch().checked_sub(1).ok_or_else(|| {
        BlockAncestryError::EpochZeroHasNoPreviousEpoch(start.compute_block_commitment())
    })?;
    let mut is_start = true;
    walk_ancestry(start, fetch, |parent, block| {
        if is_start {
            is_start = false;
            return Ok(ControlFlow::Continue(()));
        }
        let expected = if parent.is_terminal() {
            previous_epoch
        } else {
            start.epoch()
        };
        if parent.epoch() != expected {
            return Err(BlockAncestryError::UnexpectedEpoch {
                block,
                expected,
                actual: parent.epoch(),
            });
        }
        if parent.is_terminal() {
            return Ok(ControlFlow::Break(EpochCommitment::from_terminal(
                previous_epoch,
                block,
            )));
        }
        visit(block);
        Ok(ControlFlow::Continue(()))
    })
}

// Visits `start` and then follows exact-slot parent links until `visit` breaks. Every
// fetched parent must exist and hash to the requested ID. This function has no implicit
// terminal condition: the visitor must break before the selected branch has no parent.
fn walk_ancestry<T>(
    start: &OLBlockHeaderV1,
    mut fetch: impl FnMut(OLBlockId) -> DbResult<Option<OLBlockHeaderV1>>,
    mut visit: impl FnMut(
        &OLBlockHeaderV1,
        OLBlockCommitment,
    ) -> Result<ControlFlow<T>, BlockAncestryError>,
) -> Result<T, BlockAncestryError> {
    let mut current = start.clone();
    let mut commitment = start.compute_block_commitment();
    loop {
        if let ControlFlow::Break(result) = visit(&current, commitment)? {
            return Ok(result);
        }
        let parent_id = *current.parent_blkid();
        let parent = fetch(parent_id)?.ok_or(BlockAncestryError::MissingHeader(parent_id))?;
        let actual = parent.compute_blkid();
        if actual != parent_id {
            return Err(BlockAncestryError::BlockIdMismatch {
                expected: parent_id,
                actual,
            });
        }
        if parent.slot().checked_add(1) != Some(current.slot()) {
            return Err(BlockAncestryError::ParentSlotMismatch {
                child: commitment,
                parent_slot: parent.slot(),
            });
        }
        commitment = OLBlockCommitment::new(parent.slot(), parent_id);
        current = parent;
    }
}

#[cfg(test)]
mod tests {
    use std::slice;
    use std::sync::Arc;

    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_db_types::DbError;
    use strata_identifiers::EpochCommitment;
    use strata_ol_chain_types_v1::OLBlockHeaderV1;
    use strata_storage::{OLBlockManager, create_node_storage, test_runtime_handle};

    use super::{BlockAncestryProvider, collect_block_commitments, find_previous_epoch_commitment};
    use crate::test_utils::{BlockAncestryView, InvalidAncestry};
    use crate::{BlockAncestryError, EpochBlockAncestry};

    struct BlockAncestryFixture {
        view: BlockAncestryView,
        ol_block: Arc<OLBlockManager>,
        provider: BlockAncestryProvider,
    }

    impl BlockAncestryFixture {
        fn new() -> Self {
            let storage = create_node_storage(get_test_sled_backend(), test_runtime_handle())
                .expect("test: create storage");
            let ol_block = storage.ol_block().clone();
            let view = BlockAncestryView::with_storage(ol_block.clone());
            Self {
                provider: BlockAncestryProvider::new(ol_block.clone()),
                view,
                ol_block,
            }
        }

        fn genesis(&self) -> &OLBlockHeaderV1 {
            self.view.genesis()
        }

        fn append_block(&mut self, parent: &OLBlockHeaderV1) -> OLBlockHeaderV1 {
            self.view.append_block(parent)
        }

        fn append_terminal_block(&mut self, parent: &OLBlockHeaderV1) -> OLBlockHeaderV1 {
            self.view.append_terminal_block(parent)
        }

        fn append_invalid_child(
            &mut self,
            parent: &OLBlockHeaderV1,
            invalidity: InvalidAncestry,
        ) -> OLBlockHeaderV1 {
            self.view.append_invalid_child(parent, invalidity)
        }

        fn extend_chain(
            &mut self,
            parent: &OLBlockHeaderV1,
            terminal_flags: &[bool],
        ) -> Vec<OLBlockHeaderV1> {
            let mut chain = Vec::with_capacity(terminal_flags.len());
            for &terminal in terminal_flags {
                let parent = chain.last().unwrap_or(parent);
                let block = if terminal {
                    self.append_terminal_block(parent)
                } else {
                    self.append_block(parent)
                };
                chain.push(block);
            }
            chain
        }

        fn promote_history_anchor(&mut self, terminal: &OLBlockHeaderV1) -> EpochCommitment {
            self.view.promote_history_anchor(terminal)
        }

        fn drop_header(&mut self, header: &OLBlockHeaderV1) {
            self.view.drop_header(header.compute_blkid());
        }

        fn set_canonical_suffix(&self, chain: &[OLBlockHeaderV1]) {
            self.ol_block
                .replace_canonical_suffix_from_blocking(
                    chain.first().expect("test suffix must not be empty").slot(),
                    chain.iter().map(OLBlockHeaderV1::compute_blkid).collect(),
                )
                .expect("test: set canonical suffix");
        }

        fn assert_ancestry(&self, start: &OLBlockHeaderV1) {
            let expected = self.view.expected_epoch_ancestry(start);
            self.assert_ancestry_eq(start, &expected);
        }

        fn assert_ancestry_eq(&self, start: &OLBlockHeaderV1, expected: &EpochBlockAncestry) {
            assert_eq!(
                self.provider
                    .find_previous_epoch_commitment_blocking(start)
                    .expect("test: find previous epoch"),
                expected.previous_epoch()
            );
            let ancestry = self
                .provider
                .collect_epoch_block_commitments_blocking(start)
                .expect("test: collect epoch ancestry");
            assert_eq!(&ancestry, expected);

            // Both async entry points must resolve the same branch and ordering.
            let runtime = test_runtime_handle();
            assert_eq!(
                runtime
                    .block_on(self.provider.find_previous_epoch_commitment_async(start))
                    .expect("test: find previous epoch asynchronously"),
                expected.previous_epoch()
            );
            assert_eq!(
                runtime
                    .block_on(self.provider.collect_epoch_block_commitments_async(start))
                    .expect("test: collect epoch ancestry asynchronously"),
                ancestry
            );
            if start.is_terminal() {
                let epoch =
                    EpochCommitment::from_terminal(start.epoch(), start.compute_block_commitment());
                assert_eq!(
                    self.provider
                        .collect_epoch_block_commitments_at_blocking(epoch)
                        .expect("test: collect validated epoch ancestry"),
                    ancestry
                );
                assert_eq!(
                    runtime
                        .block_on(
                            self.provider
                                .collect_epoch_block_commitments_at_async(epoch),
                        )
                        .expect("test: collect validated epoch ancestry asynchronously"),
                    ancestry
                );
            }
            let (previous_epoch, blocks) = ancestry.into_parts();
            assert_eq!(previous_epoch, expected.previous_epoch());
            assert_eq!(blocks, expected.blocks());
        }
    }

    #[test]
    fn test_recent_history_crosses_epochs_to_genesis() {
        let mut fixture = BlockAncestryFixture::new();
        let genesis = fixture.genesis().clone();
        let chain = fixture.extend_chain(&genesis, &[false, true, false, true]);
        let start = chain.last().expect("test: chain has a tip");
        // The history contains four non-genesis blocks plus genesis. These counts cover an empty
        // window, partial windows, every non-genesis block, the full chain and an unbounded
        // request.
        for max_blocks in [0, 1, 2, 4, 5, usize::MAX] {
            let expected = fixture.view.expected_recent_history(start, max_blocks);
            assert_eq!(
                fixture
                    .provider
                    .collect_block_commitments_blocking(start, max_blocks)
                    .expect("test: collect recent history"),
                expected
            );
            assert_eq!(
                test_runtime_handle()
                    .block_on(
                        fixture
                            .provider
                            .collect_block_commitments_async(start, max_blocks)
                    )
                    .expect("test: collect recent history asynchronously"),
                expected
            );
        }
    }

    #[test]
    fn test_recent_history_stops_before_history_base() {
        let mut fixture = BlockAncestryFixture::new();
        let genesis = fixture.genesis().clone();
        // Grow the history base from genesis so the pruned history really exists before promotion.
        let retained = fixture.extend_chain(&genesis, &[false, true, false, true]);
        let base = retained
            .last()
            .expect("test: retained history ends at the history base");
        let chain = fixture.extend_chain(base, &[false, true, false]);
        let start = chain.last().expect("test: chain has a tip");
        fixture.promote_history_anchor(base);
        let expected = fixture.view.expected_recent_history(start, 10);

        // The history base and its pruned ancestors are excluded without reading their headers.
        assert_eq!(
            fixture
                .provider
                .collect_block_commitments_blocking(start, 10)
                .expect("test: collect retained history"),
            expected
        );
        assert_eq!(
            test_runtime_handle()
                .block_on(fixture.provider.collect_block_commitments_async(start, 10))
                .expect("test: collect retained history asynchronously"),
            expected
        );
        assert_eq!(
            fixture
                .provider
                .collect_block_commitments_blocking(base, 10)
                .expect("test: history base is excluded"),
            vec![]
        );
        assert_eq!(
            fixture
                .provider
                .collect_block_commitments_blocking(&genesis, 10)
                .expect("test: pruned start is excluded"),
            vec![]
        );
        assert_eq!(
            test_runtime_handle()
                .block_on(fixture.provider.collect_block_commitments_async(base, 10))
                .expect("test: history base is excluded asynchronously"),
            vec![]
        );
        assert_eq!(
            fixture
                .provider
                .collect_block_commitments_blocking(start, 1)
                .expect("test: count limit precedes history base"),
            vec![start.compute_block_commitment()]
        );
    }

    #[test]
    fn test_recent_history_rejects_other_branch_at_history_base() {
        let mut fixture = BlockAncestryFixture::new();
        let genesis = fixture.genesis().clone();
        let pivot = fixture.append_block(&genesis);
        let base = fixture.append_terminal_block(&pivot);
        let other = fixture.append_terminal_block(&pivot);
        let first = fixture.append_block(&other);
        let tip = fixture.append_terminal_block(&first);
        fixture.promote_history_anchor(&base);

        // The tip's branch reaches the history-base slot through `other`, not the base, so the walk
        // must refuse it.
        let expected = base.compute_blkid();
        let actual = other.compute_blkid();
        assert!(
            matches!(fixture.provider.collect_block_commitments_blocking(&tip, 10),
            Err(BlockAncestryError::UnexpectedAncestor { expected: e, actual: a }) if e == expected && a == actual)
        );
        assert!(
            matches!(test_runtime_handle().block_on(fixture.provider.collect_block_commitments_async(&tip, 10)),
            Err(BlockAncestryError::UnexpectedAncestor { expected: e, actual: a }) if e == expected && a == actual)
        );
        // The wrong branch is detected from the child's parent link alone, without reading the
        // pruned header.
        assert!(
            matches!(collect_block_commitments(&first, 10, Some(base.compute_block_commitment()), |_| panic!("history base must be checked before fetching it")),
            Err(BlockAncestryError::UnexpectedAncestor { expected: e, actual: a }) if e == expected && a == actual)
        );
        // A window that ends before reaching the history base never checks it, so it succeeds.
        assert_eq!(
            fixture
                .provider
                .collect_block_commitments_blocking(&tip, 1)
                .unwrap(),
            vec![tip.compute_block_commitment()]
        );
    }

    #[test]
    fn test_recent_history_follows_start_fork() {
        let mut fixture = BlockAncestryFixture::new();
        let genesis = fixture.genesis().clone();
        let pivot = fixture.append_block(&genesis);
        let branch_a = fixture.extend_chain(&pivot, &[true, false, true]);
        let branch_b = fixture.extend_chain(&pivot, &[false, true, true]);
        // Changing the canonical suffix must not change a walk rooted on either stored branch.
        for canonical in [&branch_a, &branch_b] {
            fixture.set_canonical_suffix(canonical);
            for branch in [&branch_a, &branch_b] {
                let start = branch.last().expect("test: branch has a tip");
                let expected = fixture.view.expected_recent_history(start, 4);
                assert_eq!(
                    fixture
                        .provider
                        .collect_block_commitments_blocking(start, 4)
                        .unwrap(),
                    expected
                );
            }
        }
    }

    #[test]
    fn test_recent_history_reads_nothing_past_stop() {
        let mut view = BlockAncestryView::new();
        let genesis = view.genesis().clone();
        let start = view.append_block(&genesis);
        // Each case stops before fetching a parent: a zero count, a count of one, a start at the
        // history base, and a parent that is the history base.
        for (max_blocks, history_base, expected) in [
            (0, None, vec![]),
            (1, None, vec![start.compute_block_commitment()]),
            (10, Some(start.compute_block_commitment()), vec![]),
            (
                10,
                Some(genesis.compute_block_commitment()),
                vec![start.compute_block_commitment()],
            ),
        ] {
            assert_eq!(
                collect_block_commitments(&start, max_blocks, history_base, |_| panic!(
                    "stop must not fetch another header"
                ))
                .expect("test: stop before fetching parent"),
                expected
            );
        }
        assert_eq!(
            collect_block_commitments(&genesis, 10, None, |_| panic!(
                "genesis has no parent to fetch"
            ))
            .expect("test: genesis stops before fetching parent"),
            vec![genesis.compute_block_commitment()]
        );
    }

    #[test]
    fn test_recent_history_fails_rather_than_truncates() {
        let mut fixture = BlockAncestryFixture::new();
        let genesis = fixture.genesis().clone();
        let missing = fixture.append_block(&genesis);
        let start = fixture.append_terminal_block(&missing);
        fixture.drop_header(&missing);

        // A missing intermediate header fails the whole request in both entry points.
        assert!(
            matches!(fixture.provider.collect_block_commitments_blocking(&start, 3), Err(BlockAncestryError::MissingHeader(id)) if id == missing.compute_blkid())
        );
        assert!(
            matches!(test_runtime_handle().block_on(fixture.provider.collect_block_commitments_async(&start, 3)), Err(BlockAncestryError::MissingHeader(id)) if id == missing.compute_blkid())
        );
    }

    #[test]
    fn test_recent_history_storage_failure_preserved() {
        let mut view = BlockAncestryView::new();
        let genesis = view.genesis().clone();
        let start = view.append_block(&genesis);
        assert!(matches!(
            collect_block_commitments(&start, 3, None, |_| Err(DbError::Busy)),
            Err(BlockAncestryError::Db(DbError::Busy))
        ));
    }

    #[test]
    fn test_recent_history_mismatched_block_id_rejected() {
        let mut view = BlockAncestryView::new();
        let genesis = view.genesis().clone();
        let requested = view.append_block(&genesis);
        let wrong = view.append_block(&genesis);
        let start = view.append_terminal_block(&requested);
        // Serve a sibling's header under `requested`'s ID. Its slot and epoch are valid, so only
        // the ID check can fail.
        view.substitute_header(requested.compute_blkid(), wrong.compute_blkid());
        assert!(
            matches!(collect_block_commitments(&start, 3, None, |id| Ok(view.visible_header(id))), Err(BlockAncestryError::BlockIdMismatch { expected, actual }) if expected == requested.compute_blkid() && actual == wrong.compute_blkid())
        );
    }

    #[test]
    fn test_recent_history_parent_slot_must_immediately_precede_child() {
        // The parent is at slot 4: child slot 6 skips a slot, 4 equals it and 3 comes before it.
        for child_slot in [6, 4, 3] {
            let mut fixture = BlockAncestryFixture::new();
            let genesis = fixture.genesis().clone();
            let chain = fixture.extend_chain(&genesis, &[false, false, false, true]);
            let parent = chain.last().expect("test: chain has a parent");
            let child =
                fixture.append_invalid_child(parent, InvalidAncestry::NonAdjacentSlot(child_slot));
            let child_commitment = child.compute_block_commitment();
            let result = fixture
                .provider
                .collect_block_commitments_blocking(&child, 3);
            assert!(
                matches!(&result, Err(BlockAncestryError::ParentSlotMismatch { child, parent_slot }) if *child == child_commitment && *parent_slot == parent.slot()),
                "child slot {child_slot}: {result:?}"
            );
        }
    }

    #[test]
    fn test_history_base_link_must_be_adjacent_slot() {
        let mut fixture = BlockAncestryFixture::new();
        let genesis = fixture.genesis().clone();
        let base = fixture.append_terminal_block(&genesis);
        let child =
            fixture.append_invalid_child(&base, InvalidAncestry::NonAdjacentSlot(base.slot() + 2));
        let child_commitment = child.compute_block_commitment();
        fixture.promote_history_anchor(&base);

        // The child names the history base as its parent but skips a slot. The base shortcut must
        // enforce adjacency without reading the history-base header.
        assert!(matches!(
            fixture.provider.collect_block_commitments_blocking(&child, 10),
            Err(BlockAncestryError::ParentSlotMismatch {
                child,
                parent_slot,
            }) if child == child_commitment && parent_slot == base.slot()
        ));
    }

    #[test]
    fn test_epoch_ancestry_lists_blocks_in_execution_order() {
        let mut fixture = BlockAncestryFixture::new();
        let genesis = fixture.genesis().clone();
        let previous = fixture.append_terminal_block(&genesis);
        let first = fixture.append_block(&previous);
        let second = fixture.append_block(&first);
        let terminal = fixture.append_terminal_block(&second);

        // The starting terminal belongs to the collected epoch, not its predecessor.
        fixture.assert_ancestry(&terminal);
    }

    #[test]
    fn test_first_epoch_ancestry_ends_at_genesis() {
        let mut fixture = BlockAncestryFixture::new();
        let genesis = fixture.genesis().clone();
        let first = fixture.append_block(&genesis);
        let second = fixture.append_block(&first);
        fixture.assert_ancestry(&second);
    }

    #[test]
    fn test_single_block_epoch() {
        let mut fixture = BlockAncestryFixture::new();
        let genesis = fixture.genesis().clone();
        let terminal = fixture.append_terminal_block(&genesis);
        fixture.assert_ancestry(&terminal);
    }

    #[test]
    fn test_missing_epoch_terminal_rejected() {
        let mut fixture = BlockAncestryFixture::new();
        let genesis = fixture.genesis().clone();
        let terminal = fixture.append_terminal_block(&genesis);
        let epoch =
            EpochCommitment::from_terminal(terminal.epoch(), terminal.compute_block_commitment());
        fixture.drop_header(&terminal);

        assert!(matches!(
            fixture.provider.collect_epoch_block_commitments_at_blocking(epoch),
            Err(BlockAncestryError::MissingEpochTerminal(missing)) if missing == epoch
        ));
        assert!(matches!(
            test_runtime_handle().block_on(
                fixture.provider.collect_epoch_block_commitments_at_async(epoch)
            ),
            Err(BlockAncestryError::MissingEpochTerminal(missing)) if missing == epoch
        ));
    }

    #[test]
    fn test_epoch_terminal_commitment_mismatch_rejected() {
        let mut fixture = BlockAncestryFixture::new();
        let genesis = fixture.genesis().clone();
        let terminal = fixture.append_terminal_block(&genesis);
        let actual =
            EpochCommitment::from_terminal(terminal.epoch(), terminal.compute_block_commitment());
        let expected = EpochCommitment::from_terminal(
            terminal.epoch() + 1,
            terminal.compute_block_commitment(),
        );

        assert!(matches!(
            fixture.provider.collect_epoch_block_commitments_at_blocking(expected),
            Err(BlockAncestryError::EpochTerminalMismatch {
                expected: error_expected,
                actual: error_actual,
            }) if error_expected == expected && error_actual == actual
        ));
    }

    #[test]
    fn test_nonterminal_epoch_commitment_rejected() {
        let mut fixture = BlockAncestryFixture::new();
        let genesis = fixture.genesis().clone();
        let block = fixture.append_block(&genesis);
        let epoch = EpochCommitment::from_terminal(block.epoch(), block.compute_block_commitment());

        assert!(matches!(
            fixture.provider.collect_epoch_block_commitments_at_blocking(epoch),
            Err(BlockAncestryError::MissingTerminalFlag(actual)) if actual == epoch
        ));
    }

    #[test]
    fn test_previous_terminal_mismatch_rejected() {
        let mut fixture = BlockAncestryFixture::new();
        let genesis = fixture.genesis().clone();
        let previous = fixture.append_terminal_block(&genesis);
        let terminal = fixture.append_terminal_block(&previous);
        let ancestry = fixture
            .provider
            .collect_epoch_block_commitments_blocking(&terminal)
            .expect("test: collect ancestry");
        let expected = genesis.compute_block_commitment();
        let actual = previous.compute_block_commitment();

        assert!(matches!(
            ancestry.require_previous_terminal(expected),
            Err(BlockAncestryError::PreviousTerminalMismatch {
                expected: error_expected,
                actual: error_actual,
            }) if error_expected == expected && error_actual == actual
        ));
    }

    #[test]
    fn test_previous_terminal_resolves_from_retained_header() {
        let mut fixture = BlockAncestryFixture::new();
        let genesis = fixture.genesis().clone();
        let previous = fixture.append_terminal_block(&genesis);
        let first = fixture.append_block(&previous);
        let start = fixture.append_terminal_block(&first);

        // Promotion retains only the history-base header from previous history, so the walk
        // must stop at the history base without reading anything older.
        fixture.promote_history_anchor(&previous);
        fixture.assert_ancestry(&start);
    }

    #[test]
    fn test_ancestry_resolves_from_start_block_not_canonical_index() {
        let mut fixture = BlockAncestryFixture::new();
        // Fork inside epoch 1 after a shared prefix. A ends epoch 1 at slot 4;
        // B ends it at slot 5. Both continue through epoch 2 to terminal tips.
        let genesis = fixture.genesis().clone();
        let shared = fixture.extend_chain(&genesis, &[false, false]);
        let pivot = &shared[1];
        let branch_a = fixture.extend_chain(pivot, &[false, true, false, false, true]);
        let branch_b = fixture.extend_chain(pivot, &[false, false, true, false, true]);
        fixture.set_canonical_suffix(slice::from_ref(&genesis));
        fixture.set_canonical_suffix(&shared);

        // Check both branches with A canonical, then replace only the suffix
        // after the pivot with B. Neither lookup nor collection may change branches.
        for canonical_branch in [&branch_a, &branch_b] {
            fixture.set_canonical_suffix(canonical_branch);
            for (branch, terminal_index) in [(&branch_a, 1), (&branch_b, 2)] {
                let current_epoch = &branch[terminal_index + 1..];
                // Exercise a nonterminal start and a terminal start on each branch.
                fixture.assert_ancestry(&current_epoch[0]);
                fixture
                    .assert_ancestry(current_epoch.last().expect("test: current epoch has a tip"));

                // Searching from the epoch-1 terminal crosses the pivot and
                // includes the shared prefix exactly once, ending at genesis.
                fixture.assert_ancestry(&branch[terminal_index]);
            }
        }
    }

    #[test]
    fn test_epoch_ancestry_ignores_missing_headers_on_other_branches() {
        let mut fixture = BlockAncestryFixture::new();
        let genesis = fixture.genesis().clone();
        let previous = fixture.append_terminal_block(&genesis);
        let first = fixture.append_block(&previous);
        let start = fixture.append_terminal_block(&first);
        // A sibling of `first` forks inside the same epoch, then loses its header.
        let sibling = fixture.append_block(&previous);
        fixture.drop_header(&sibling);

        // The walk reads only `start`'s parent links, so the gap on the other branch
        // leaves the recorded ancestry intact.
        fixture.assert_ancestry(&start);
    }

    #[test]
    fn test_missing_intermediate_header_errors() {
        let mut fixture = BlockAncestryFixture::new();
        let genesis = fixture.genesis().clone();
        let missing = fixture.append_block(&genesis);
        let start = fixture.append_terminal_block(&missing);
        let missing_id = missing.compute_blkid();
        fixture.drop_header(&missing);

        assert!(matches!(
            fixture.provider.collect_epoch_block_commitments_blocking(&start),
            Err(BlockAncestryError::MissingHeader(id)) if id == missing_id
        ));
        assert!(matches!(
            test_runtime_handle().block_on(fixture.provider.find_previous_epoch_commitment_async(&start)),
            Err(BlockAncestryError::MissingHeader(id)) if id == missing_id
        ));
    }

    #[test]
    fn test_epoch_zero_has_no_previous_epoch() {
        let fixture = BlockAncestryFixture::new();
        let genesis = fixture.genesis();
        assert!(matches!(
            fixture.provider.find_previous_epoch_commitment_blocking(genesis),
            Err(BlockAncestryError::EpochZeroHasNoPreviousEpoch(block))
                if block == genesis.compute_block_commitment()
        ));
    }

    #[test]
    fn test_invalid_epoch_boundaries_rejected() {
        // A start in epoch 3 requires nonterminal parents in 3 and a terminal in 2.
        // Inject a skipped epoch, same-epoch terminal, nonterminal boundary, and future epoch.
        for (parent_epoch, terminal, expected) in
            [(1, true, 2), (3, true, 2), (2, false, 3), (4, false, 3)]
        {
            let mut fixture = BlockAncestryFixture::new();
            let genesis = fixture.genesis().clone();
            let prefix = fixture.extend_chain(&genesis, &[false, false, false, false]);
            let prefix_tip = prefix.last().expect("test: prefix has a tip");
            // A parent in its derived epoch is valid, so use an ordinary append. The invalid
            // ancestry API refuses consistent epoch metadata.
            let parent = if parent_epoch == prefix_tip.epoch() {
                if terminal {
                    fixture.append_terminal_block(prefix_tip)
                } else {
                    fixture.append_block(prefix_tip)
                }
            } else {
                fixture.append_invalid_child(
                    prefix_tip,
                    InvalidAncestry::EpochMetadata {
                        epoch: parent_epoch,
                        terminal,
                    },
                )
            };
            let start = fixture.append_invalid_child(
                &parent,
                InvalidAncestry::EpochMetadata {
                    epoch: 3,
                    terminal: true,
                },
            );
            assert!(matches!(
                fixture.provider.find_previous_epoch_commitment_blocking(&start),
                Err(BlockAncestryError::UnexpectedEpoch { block, expected: actual_expected, actual })
                    if block == parent.compute_block_commitment()
                        && actual_expected == expected && actual == parent_epoch
            ));
        }
    }

    #[test]
    fn test_parent_slot_must_immediately_precede_child() {
        // The parent is at slot 4: child slot 6 skips a slot, 4 equals it and 3 comes before it.
        for child_slot in [6, 4, 3] {
            let mut fixture = BlockAncestryFixture::new();
            let genesis = fixture.genesis().clone();
            let prefix = fixture.extend_chain(&genesis, &[false, false, false, true]);
            let parent = prefix.last().expect("test: prefix has a terminal");
            let start =
                fixture.append_invalid_child(parent, InvalidAncestry::NonAdjacentSlot(child_slot));
            let start_commitment = start.compute_block_commitment();
            let result = fixture
                .provider
                .find_previous_epoch_commitment_blocking(&start);
            assert!(
                matches!(&result, Err(BlockAncestryError::ParentSlotMismatch { child, parent_slot }) if *child == start_commitment && *parent_slot == parent.slot()),
                "child slot {child_slot}: {result:?}"
            );
        }
    }

    #[test]
    fn test_mismatched_block_id_rejected() {
        // Substitute another branch's header at either the intermediate block
        // or the terminal boundary. Both have valid slots and epochs.
        for substitute_previous in [false, true] {
            let mut view = BlockAncestryView::new();
            let genesis = view.genesis().clone();
            let previous = view.append_terminal_block(&genesis);
            let wrong_previous = view.append_terminal_block(&genesis);
            let first = view.append_block(&previous);
            let wrong_first = view.append_block(&previous);
            let start = view.append_terminal_block(&first);
            let (requested, wrong) = if substitute_previous {
                (&previous, &wrong_previous)
            } else {
                (&first, &wrong_first)
            };
            let expected = requested.compute_blkid();
            let actual = wrong.compute_blkid();
            view.substitute_header(expected, actual);
            let result =
                find_previous_epoch_commitment(&start, |id| Ok(view.visible_header(id)), |_| {});

            assert!(matches!(
                result,
                Err(BlockAncestryError::BlockIdMismatch { expected: key, actual: hash })
                    if key == expected && hash == actual
            ));
        }
    }

    #[test]
    fn test_storage_failure_preserved() {
        let mut view = BlockAncestryView::new();
        let genesis = view.genesis().clone();
        let start = view.append_terminal_block(&genesis);
        // Fail the header read before ancestry can be inspected.
        let result = find_previous_epoch_commitment(
            &start,
            |_| Err(DbError::Busy),
            |_| panic!("a failed header read must not visit a block"),
        );
        assert!(matches!(result, Err(BlockAncestryError::Db(DbError::Busy))));
    }
}
