use std::ops::ControlFlow;
use std::sync::Arc;

use strata_db_types::{DbError, DbResult};
use strata_identifiers::{OLBlockCommitment, OLBlockId};
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
    blocks: Arc<OLBlockManager>,
}

/// An epoch's blocks in execution order and the preceding terminal block.
#[derive(Debug, PartialEq, Eq)]
pub struct EpochBlockAncestry {
    previous_terminal: OLBlockCommitment,
    blocks: Vec<OLBlockCommitment>,
}

impl EpochBlockAncestry {
    /// Returns the previous epoch's terminal block.
    pub fn previous_terminal(&self) -> &OLBlockCommitment {
        &self.previous_terminal
    }

    /// Returns the nonempty sequence through the starting block, excluding the previous terminal.
    pub fn blocks(&self) -> &[OLBlockCommitment] {
        &self.blocks
    }

    /// Takes the previous terminal and the blocks in execution order.
    pub fn into_parts(self) -> (OLBlockCommitment, Vec<OLBlockCommitment>) {
        (self.previous_terminal, self.blocks)
    }
}

impl BlockAncestryProvider {
    /// Creates a provider backed by the block manager.
    pub fn new(blocks: Arc<OLBlockManager>) -> Self {
        Self { blocks }
    }

    /// Finds the previous epoch's terminal along `start`'s parent chain.
    ///
    /// Excludes `start`, even when terminal. Epoch zero has no previous epoch.
    /// Missing headers or inconsistent slot/epoch boundaries return an error.
    /// A retained terminal header suffices; its body and ancestors are not required.
    pub fn find_previous_epoch_terminal_block_blocking(
        &self,
        start: &OLBlockHeaderV1,
    ) -> Result<OLBlockCommitment, BlockAncestryError> {
        find_previous_epoch_terminal_block(
            start,
            |id| self.blocks.get_ol_header_blocking(id),
            |_| {},
        )
    }

    /// Async counterpart of [`Self::find_previous_epoch_terminal_block_blocking`].
    pub async fn find_previous_epoch_terminal_block_async(
        &self,
        start: &OLBlockHeaderV1,
    ) -> Result<OLBlockCommitment, BlockAncestryError> {
        let provider = self.clone();
        let start = start.clone();
        spawn_blocking(move || provider.find_previous_epoch_terminal_block_blocking(&start))
            .await
            .map_err(DbError::from)?
    }

    /// Collects current-epoch commitments through `start` in execution order.
    ///
    /// Includes `start` and excludes the previous terminal, which is returned separately.
    /// Uses the boundary and failure rules of
    /// [`Self::find_previous_epoch_terminal_block_blocking`].
    pub fn collect_epoch_block_commitments_blocking(
        &self,
        start: &OLBlockHeaderV1,
    ) -> Result<EpochBlockAncestry, BlockAncestryError> {
        let mut blocks = vec![start.compute_block_commitment()];
        let previous_terminal = find_previous_epoch_terminal_block(
            start,
            |id| self.blocks.get_ol_header_blocking(id),
            |block| blocks.push(block),
        )?;
        blocks.reverse();
        Ok(EpochBlockAncestry {
            previous_terminal,
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

    /// Collects up to `max_blocks` commitments by following parents from `start`,
    /// returning them in ascending slot order.
    ///
    /// Includes `start` and crosses epoch boundaries. Stops at the limit, genesis
    /// (included), or the stored history base (excluded). Returns an empty list
    /// when the limit is zero or `start` is at or below the history base.
    ///
    /// Missing or inconsistent ancestry and storage failures return an error,
    /// never a partial list. Assumes the supplied starting header is valid.
    pub fn collect_block_commitments_blocking(
        &self,
        start: &OLBlockHeaderV1,
        max_blocks: usize,
    ) -> Result<Vec<OLBlockCommitment>, BlockAncestryError> {
        // A zero limit needs no storage reads, including the history-base lookup.
        if max_blocks == 0 {
            return Ok(Vec::new());
        }
        // Read the boundary once for this walk. Its retained header does not imply
        // a retained body, so collection excludes the boundary itself.
        let history_base = self
            .blocks
            .get_history_base_blocking()?
            .map(|base| base.to_block_commitment());
        collect_block_commitments(start, max_blocks, history_base, |id| {
            self.blocks.get_ol_header_blocking(id)
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
        if let Some(base) = history_base.filter(|base| block.slot() <= base.slot()) {
            if block.blkid() != base.blkid() {
                return Err(BlockAncestryError::UnexpectedAncestor {
                    expected: *base.blkid(),
                    actual: *block.blkid(),
                });
            }
            return Ok(ControlFlow::Break(()));
        }
        blocks.push(block);
        if blocks.len() == max_blocks || header.slot() == 0 {
            return Ok(ControlFlow::Break(()));
        }
        if let Some(base) = history_base {
            // At the adjacent slot, only the retained anchor can end the walk.
            // Check its ID without requiring the pruned boundary's header.
            if header.slot().checked_sub(1) == Some(base.slot())
                && header.parent_blkid() != base.blkid()
            {
                return Err(BlockAncestryError::UnexpectedAncestor {
                    expected: *base.blkid(),
                    actual: *header.parent_blkid(),
                });
            }
            if header.parent_blkid() == base.blkid() {
                return Ok(ControlFlow::Break(()));
            }
        }
        Ok(ControlFlow::Continue(()))
    })?;
    blocks.reverse();
    Ok(blocks)
}

fn find_previous_epoch_terminal_block(
    start: &OLBlockHeaderV1,
    fetch: impl FnMut(OLBlockId) -> DbResult<Option<OLBlockHeaderV1>>,
    mut visit: impl FnMut(OLBlockCommitment),
) -> Result<OLBlockCommitment, BlockAncestryError> {
    let previous_epoch = start
        .epoch()
        .checked_sub(1)
        .ok_or_else(|| BlockAncestryError::NoPreviousEpoch(start.compute_block_commitment()))?;
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
            return Ok(ControlFlow::Break(block));
        }
        visit(block);
        Ok(ControlFlow::Continue(()))
    })
}

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
        if parent.slot() >= current.slot() {
            return Err(BlockAncestryError::ParentSlotNotDescending {
                block: OLBlockCommitment::new(parent.slot(), parent_id),
                parent_slot: parent.slot(),
                child_slot: current.slot(),
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
    use strata_identifiers::{
        Buf32, Buf64, Epoch, EpochCommitment, OLBlockCommitment, OLBlockId, Slot,
    };
    use strata_ol_chain_types_v1::{
        BlockFlagsV1, OLBlockBodyV1, OLBlockHeaderV1, OLBlockV1, OLTxSegmentV1,
        SignedOLBlockHeaderV1,
    };
    use strata_storage::{OLBlockManager, create_node_storage, test_runtime_handle};

    use super::{
        BlockAncestryProvider, collect_block_commitments, find_previous_epoch_terminal_block,
    };
    use crate::BlockAncestryError;

    struct BlockAncestryFixture {
        blocks: Arc<OLBlockManager>,
        provider: BlockAncestryProvider,
        genesis: OLBlockHeaderV1,
    }

    impl BlockAncestryFixture {
        fn new() -> Self {
            let storage =
                create_node_storage(get_test_sled_backend(), test_runtime_handle()).unwrap();
            let blocks = storage.ol_block().clone();
            let genesis = make_block(0, 0, OLBlockId::from(Buf32::zero()), true, 0);
            blocks.put_block_data_blocking(genesis.clone()).unwrap();
            Self {
                provider: BlockAncestryProvider::new(blocks.clone()),
                blocks,
                genesis: genesis.header().clone(),
            }
        }

        fn append_block(
            &self,
            parent: &OLBlockHeaderV1,
            terminal: bool,
            branch: u8,
        ) -> OLBlockHeaderV1 {
            let block = make_block(
                parent.slot() + 1,
                parent.epoch() + u32::from(parent.is_terminal()),
                parent.compute_blkid(),
                terminal,
                branch,
            );
            self.blocks.put_block_data_blocking(block.clone()).unwrap();
            block.header().clone()
        }

        fn extend_chain(
            &self,
            parent: &OLBlockHeaderV1,
            terminal_flags: &[bool],
            branch: u8,
        ) -> Vec<OLBlockHeaderV1> {
            let mut chain = Vec::with_capacity(terminal_flags.len());
            for &terminal in terminal_flags {
                let parent = chain.last().unwrap_or(parent);
                chain.push(self.append_block(parent, terminal, branch));
            }
            chain
        }

        fn set_canonical_suffix(&self, chain: &[OLBlockHeaderV1]) {
            self.blocks
                .replace_canonical_suffix_from_blocking(
                    chain.first().expect("test suffix must not be empty").slot(),
                    chain.iter().map(OLBlockHeaderV1::compute_blkid).collect(),
                )
                .unwrap();
        }

        fn assert_ancestry(
            &self,
            start: &OLBlockHeaderV1,
            previous: &OLBlockHeaderV1,
            expected_blocks: &[OLBlockCommitment],
        ) {
            let terminal = previous.compute_block_commitment();
            assert_eq!(
                self.provider
                    .find_previous_epoch_terminal_block_blocking(start)
                    .unwrap(),
                terminal
            );
            let ancestry = self
                .provider
                .collect_epoch_block_commitments_blocking(start)
                .unwrap();
            assert_eq!(*ancestry.previous_terminal(), terminal);
            assert_eq!(ancestry.blocks(), expected_blocks);

            // Both async entry points must resolve the same branch and ordering.
            let runtime = test_runtime_handle();
            assert_eq!(
                runtime
                    .block_on(
                        self.provider
                            .find_previous_epoch_terminal_block_async(start)
                    )
                    .unwrap(),
                terminal
            );
            assert_eq!(
                runtime
                    .block_on(self.provider.collect_epoch_block_commitments_async(start))
                    .unwrap(),
                ancestry
            );
            assert_eq!(ancestry.into_parts(), (terminal, expected_blocks.to_vec()));
        }
    }

    fn make_block(
        slot: Slot,
        epoch: Epoch,
        parent: OLBlockId,
        terminal: bool,
        branch: u8,
    ) -> OLBlockV1 {
        let body = OLBlockBodyV1::new_common(OLTxSegmentV1::new(vec![]).unwrap());
        let mut flags = BlockFlagsV1::zero();
        flags.set_is_terminal(terminal);
        let header = OLBlockHeaderV1::new(
            1_000 + slot,
            flags,
            slot,
            epoch,
            parent,
            body.compute_hash_commitment(),
            Buf32::from([branch; 32]),
            Buf32::zero(),
        );
        OLBlockV1::new(SignedOLBlockHeaderV1::new(header, Buf64::zero()), body)
    }

    #[test]
    fn test_bounded_collection_crosses_epochs_and_caps_at_genesis() {
        let fixture = BlockAncestryFixture::new();
        let chain = fixture.extend_chain(&fixture.genesis, &[false, true, false, true], 1);
        let start = chain.last().unwrap();
        let all: Vec<_> = slice::from_ref(&fixture.genesis)
            .iter()
            .chain(&chain)
            .map(OLBlockHeaderV1::compute_block_commitment)
            .collect();
        for max_blocks in [0, 1, 2, 4, 5, usize::MAX] {
            let expected = &all[all.len().saturating_sub(max_blocks)..];
            assert_eq!(
                fixture
                    .provider
                    .collect_block_commitments_blocking(start, max_blocks)
                    .unwrap(),
                expected
            );
            assert_eq!(
                test_runtime_handle()
                    .block_on(
                        fixture
                            .provider
                            .collect_block_commitments_async(start, max_blocks)
                    )
                    .unwrap(),
                expected
            );
        }
    }

    #[test]
    fn test_bounded_collection_stops_before_unavailable_boundary() {
        let fixture = BlockAncestryFixture::new();
        // The retained boundary need not be readable: neither it nor older history is fetched.
        let boundary = make_block(50, 9, OLBlockId::from(Buf32::from([9; 32])), true, 1);
        let chain = fixture.extend_chain(boundary.header(), &[false, true, false], 1);
        fixture
            .blocks
            .promote_to_history_anchor_blocking(EpochCommitment::from_terminal(
                boundary.header().epoch(),
                boundary.header().compute_block_commitment(),
            ))
            .unwrap();
        let expected: Vec<_> = chain
            .iter()
            .map(OLBlockHeaderV1::compute_block_commitment)
            .collect();
        let start = chain.last().unwrap();
        assert_eq!(
            fixture
                .provider
                .collect_block_commitments_blocking(start, 10)
                .unwrap(),
            expected
        );
        assert_eq!(
            test_runtime_handle()
                .block_on(fixture.provider.collect_block_commitments_async(start, 10))
                .unwrap(),
            expected
        );
        assert_eq!(
            fixture
                .provider
                .collect_block_commitments_blocking(boundary.header(), 10)
                .unwrap(),
            vec![]
        );
        assert_eq!(
            fixture
                .provider
                .collect_block_commitments_blocking(&fixture.genesis, 10)
                .unwrap(),
            vec![]
        );
        assert_eq!(
            test_runtime_handle()
                .block_on(
                    fixture
                        .provider
                        .collect_block_commitments_async(boundary.header(), 10)
                )
                .unwrap(),
            vec![]
        );
        assert_eq!(
            fixture
                .provider
                .collect_block_commitments_blocking(start, 1)
                .unwrap(),
            vec![start.compute_block_commitment()]
        );
    }

    #[test]
    fn test_bounded_collection_rejects_another_branch_at_history_base() {
        let fixture = BlockAncestryFixture::new();
        let pivot = fixture.append_block(&fixture.genesis, false, 0);
        let anchor = fixture.append_block(&pivot, true, 1);
        let other = fixture.append_block(&pivot, true, 2);
        let first = fixture.append_block(&other, false, 2);
        let tip = fixture.append_block(&first, true, 2);
        fixture
            .blocks
            .promote_to_history_anchor_blocking(EpochCommitment::from_terminal(
                anchor.epoch(),
                anchor.compute_block_commitment(),
            ))
            .unwrap();

        let expected = anchor.compute_blkid();
        let actual = other.compute_blkid();
        assert!(
            matches!(fixture.provider.collect_block_commitments_blocking(&tip, 10),
            Err(BlockAncestryError::UnexpectedAncestor { expected: e, actual: a }) if e == expected && a == actual)
        );
        assert!(
            matches!(test_runtime_handle().block_on(fixture.provider.collect_block_commitments_async(&tip, 10)),
            Err(BlockAncestryError::UnexpectedAncestor { expected: e, actual: a }) if e == expected && a == actual)
        );
        // The boundary check requires no read of the excluded parent header.
        assert!(
            matches!(collect_block_commitments(&first, 10, Some(anchor.compute_block_commitment()), |_| panic!("history boundary must be checked before fetching it")),
            Err(BlockAncestryError::UnexpectedAncestor { expected: e, actual: a }) if e == expected && a == actual)
        );
        // A count limit reached before the boundary does not establish anchor ancestry.
        assert_eq!(
            fixture
                .provider
                .collect_block_commitments_blocking(&tip, 1)
                .unwrap(),
            vec![tip.compute_block_commitment()]
        );
    }

    #[test]
    fn test_bounded_collection_follows_selected_fork() {
        let fixture = BlockAncestryFixture::new();
        let pivot = fixture.append_block(&fixture.genesis, false, 0);
        let branch_a = fixture.extend_chain(&pivot, &[true, false, true], 1);
        let branch_b = fixture.extend_chain(&pivot, &[false, true, true], 2);
        for canonical in [&branch_a, &branch_b] {
            fixture.set_canonical_suffix(canonical);
            for branch in [&branch_a, &branch_b] {
                let expected: Vec<_> = slice::from_ref(&pivot)
                    .iter()
                    .chain(branch.iter())
                    .map(OLBlockHeaderV1::compute_block_commitment)
                    .collect();
                assert_eq!(
                    fixture
                        .provider
                        .collect_block_commitments_blocking(branch.last().unwrap(), 4)
                        .unwrap(),
                    expected
                );
            }
        }
    }

    #[test]
    fn test_bounded_collection_does_not_read_beyond_a_stop() {
        let start = make_block(1, 1, OLBlockId::from(Buf32::from([9; 32])), false, 1);
        for (max_blocks, history_base, expected) in [
            (0, None, vec![]),
            (1, None, vec![start.header().compute_block_commitment()]),
            (10, Some(start.header().compute_block_commitment()), vec![]),
            (
                10,
                Some(OLBlockCommitment::new(0, *start.header().parent_blkid())),
                vec![start.header().compute_block_commitment()],
            ),
        ] {
            assert_eq!(
                collect_block_commitments(start.header(), max_blocks, history_base, |_| panic!(
                    "stop must not fetch another header"
                ))
                .unwrap(),
                expected
            );
        }
        let genesis = make_block(0, 0, OLBlockId::from(Buf32::zero()), true, 0);
        assert_eq!(
            collect_block_commitments(genesis.header(), 10, None, |_| panic!(
                "genesis has no parent to fetch"
            ))
            .unwrap(),
            vec![genesis.header().compute_block_commitment()]
        );
    }

    #[test]
    fn test_bounded_collection_errors_instead_of_returning_partial_history() {
        let fixture = BlockAncestryFixture::new();
        let missing = make_block(1, 1, fixture.genesis.compute_blkid(), false, 1);
        let start = fixture.append_block(missing.header(), true, 1);
        assert!(
            matches!(fixture.provider.collect_block_commitments_blocking(&start, 3), Err(BlockAncestryError::MissingHeader(id)) if id == missing.header().compute_blkid())
        );
        assert!(
            matches!(test_runtime_handle().block_on(fixture.provider.collect_block_commitments_async(&start, 3)), Err(BlockAncestryError::MissingHeader(id)) if id == missing.header().compute_blkid())
        );
        assert!(matches!(
            collect_block_commitments(&start, 3, None, |_| Err(DbError::Busy)),
            Err(BlockAncestryError::Db(DbError::Busy))
        ));
        let wrong = make_block(1, 1, fixture.genesis.compute_blkid(), false, 2);
        assert!(
            matches!(collect_block_commitments(&start, 3, None, |_| Ok(Some(wrong.header().clone()))), Err(BlockAncestryError::BlockIdMismatch { expected, actual }) if expected == missing.header().compute_blkid() && actual == wrong.header().compute_blkid())
        );
        for slot in [start.slot(), start.slot() + 1] {
            let parent = make_block(slot, 1, fixture.genesis.compute_blkid(), false, 1);
            let child = make_block(start.slot(), 1, parent.header().compute_blkid(), false, 1);
            assert!(
                matches!(collect_block_commitments(child.header(), 3, None, |_| Ok(Some(parent.header().clone()))), Err(BlockAncestryError::ParentSlotNotDescending { parent_slot, child_slot, .. }) if parent_slot == slot && child_slot == start.slot())
            );
        }
    }

    #[test]
    fn test_terminal_start_finds_previous_epoch_and_collects_execution_order() {
        let fixture = BlockAncestryFixture::new();
        let previous = fixture.append_block(&fixture.genesis, true, 1);
        let first = fixture.append_block(&previous, false, 1);
        let second = fixture.append_block(&first, false, 1);
        let terminal = fixture.append_block(&second, true, 1);

        // The starting terminal belongs to the collected epoch, not its predecessor.
        fixture.assert_ancestry(
            &terminal,
            &previous,
            &[
                first.compute_block_commitment(),
                second.compute_block_commitment(),
                terminal.compute_block_commitment(),
            ],
        );
    }

    #[test]
    fn test_nonterminal_start_resolves_genesis_boundary() {
        let fixture = BlockAncestryFixture::new();
        let first = fixture.append_block(&fixture.genesis, false, 1);
        let second = fixture.append_block(&first, false, 1);
        fixture.assert_ancestry(
            &second,
            &fixture.genesis,
            &[
                first.compute_block_commitment(),
                second.compute_block_commitment(),
            ],
        );
    }

    #[test]
    fn test_single_block_epoch() {
        let fixture = BlockAncestryFixture::new();
        let terminal = fixture.append_block(&fixture.genesis, true, 1);
        fixture.assert_ancestry(
            &terminal,
            &fixture.genesis,
            &[terminal.compute_block_commitment()],
        );
    }

    #[test]
    fn test_retained_terminal_needs_no_body_or_older_ancestors() {
        let fixture = BlockAncestryFixture::new();
        // Only the boundary header is retained. Its parent and body are unavailable.
        let previous = make_block(50, 9, OLBlockId::from(Buf32::from([9; 32])), true, 1);
        fixture
            .blocks
            .put_terminal_header_blocking(
                previous.header().compute_blkid(),
                previous.header().clone(),
            )
            .unwrap();
        let first = fixture.append_block(previous.header(), false, 1);
        // The supplied starting header need not have been persisted yet.
        let start = make_block(52, 10, first.compute_blkid(), true, 1);
        fixture.assert_ancestry(
            start.header(),
            previous.header(),
            &[
                first.compute_block_commitment(),
                start.header().compute_block_commitment(),
            ],
        );
    }

    #[test]
    fn test_follows_supplied_branch_independently_of_canonical_index() {
        let fixture = BlockAncestryFixture::new();
        // Fork inside epoch 1 after a shared prefix. A ends epoch 1 at slot 4;
        // B ends it at slot 5. Both continue through epoch 2 to terminal tips.
        let shared = fixture.extend_chain(&fixture.genesis, &[false, false], 0);
        let pivot = &shared[1];
        let branch_a = fixture.extend_chain(pivot, &[false, true, false, false, true], 1);
        let branch_b = fixture.extend_chain(pivot, &[false, false, true, false, true], 2);
        fixture.set_canonical_suffix(slice::from_ref(&fixture.genesis));
        fixture.set_canonical_suffix(&shared);

        // Check both branches with A canonical, then replace only the suffix
        // after the pivot with B. Neither lookup nor collection may change branches.
        for canonical_branch in [&branch_a, &branch_b] {
            fixture.set_canonical_suffix(canonical_branch);
            for (branch, terminal_index) in [(&branch_a, 1), (&branch_b, 2)] {
                let previous_terminal = &branch[terminal_index];
                let current_epoch = &branch[terminal_index + 1..];
                let current_commitments: Vec<_> = current_epoch
                    .iter()
                    .map(OLBlockHeaderV1::compute_block_commitment)
                    .collect();
                // Exercise a nonterminal start and a terminal start on each branch.
                fixture.assert_ancestry(
                    &current_epoch[0],
                    previous_terminal,
                    &current_commitments[..1],
                );
                fixture.assert_ancestry(
                    current_epoch.last().unwrap(),
                    previous_terminal,
                    &current_commitments,
                );

                // Searching from the epoch-1 terminal crosses the pivot and
                // includes the shared prefix exactly once, ending at genesis.
                let first_epoch: Vec<_> = shared
                    .iter()
                    .chain(&branch[..=terminal_index])
                    .map(OLBlockHeaderV1::compute_block_commitment)
                    .collect();
                fixture.assert_ancestry(previous_terminal, &fixture.genesis, &first_epoch);
            }
        }
    }

    #[test]
    fn test_missing_intermediate_header_errors() {
        let fixture = BlockAncestryFixture::new();
        let missing = make_block(1, 1, fixture.genesis.compute_blkid(), false, 1);
        let start = fixture.append_block(missing.header(), true, 1);
        let missing_id = missing.header().compute_blkid();
        assert!(matches!(
            fixture.provider.collect_epoch_block_commitments_blocking(&start),
            Err(BlockAncestryError::MissingHeader(id)) if id == missing_id
        ));
        assert!(matches!(
            test_runtime_handle().block_on(fixture.provider.find_previous_epoch_terminal_block_async(&start)),
            Err(BlockAncestryError::MissingHeader(id)) if id == missing_id
        ));
    }

    #[test]
    fn test_epoch_zero_has_no_previous_epoch() {
        let fixture = BlockAncestryFixture::new();
        assert!(matches!(
            fixture.provider.find_previous_epoch_terminal_block_blocking(&fixture.genesis),
            Err(BlockAncestryError::NoPreviousEpoch(block))
                if block == fixture.genesis.compute_block_commitment()
        ));
    }

    #[test]
    fn test_invalid_epoch_boundaries_rejected() {
        let fixture = BlockAncestryFixture::new();
        // A start in epoch 3 requires nonterminal parents in 3 and a terminal in 2.
        // Inject a skipped epoch, same-epoch terminal, nonterminal boundary, and future epoch.
        for (parent_epoch, terminal, expected) in
            [(1, true, 2), (3, true, 2), (2, false, 3), (4, false, 3)]
        {
            let parent = make_block(
                5,
                parent_epoch,
                fixture.genesis.compute_blkid(),
                terminal,
                1,
            );
            fixture
                .blocks
                .put_block_data_blocking(parent.clone())
                .unwrap();
            let start = make_block(6, 3, parent.header().compute_blkid(), true, 1);
            assert!(matches!(
                fixture.provider.find_previous_epoch_terminal_block_blocking(start.header()),
                Err(BlockAncestryError::UnexpectedEpoch { block, expected: actual_expected, actual })
                    if block == parent.header().compute_block_commitment()
                        && actual_expected == expected && actual == parent_epoch
            ));
        }
    }

    #[test]
    fn test_parent_slots_must_strictly_decrease() {
        let fixture = BlockAncestryFixture::new();
        for parent_slot in [5, 6] {
            let parent = make_block(parent_slot, 1, fixture.genesis.compute_blkid(), true, 1);
            fixture
                .blocks
                .put_block_data_blocking(parent.clone())
                .unwrap();
            let start = make_block(5, 2, parent.header().compute_blkid(), false, 1);
            assert!(matches!(
                fixture.provider.find_previous_epoch_terminal_block_blocking(start.header()),
                Err(BlockAncestryError::ParentSlotNotDescending { parent_slot: actual, child_slot: 5, .. })
                    if actual == parent_slot
            ));
        }
    }

    #[test]
    fn test_mismatched_block_id_rejected() {
        let fixture = BlockAncestryFixture::new();
        let previous = fixture.append_block(&fixture.genesis, true, 1);
        let first = fixture.append_block(&previous, false, 1);
        let start = fixture.append_block(&first, true, 1);

        // Substitute another branch's header at either the intermediate block
        // or the terminal boundary. Both have valid slots and epochs.
        for (requested, terminal) in [(&first, false), (&previous, true)] {
            let wrong = make_block(
                requested.slot(),
                requested.epoch(),
                *requested.parent_blkid(),
                terminal,
                2,
            );
            let expected = requested.compute_blkid();
            let actual = wrong.header().compute_blkid();
            let mut visited = Vec::new();
            let result = find_previous_epoch_terminal_block(
                &start,
                |id| {
                    if id == expected {
                        Ok(Some(wrong.header().clone()))
                    } else {
                        fixture.blocks.get_ol_header_blocking(id)
                    }
                },
                |block| visited.push(block),
            );

            assert!(matches!(
                result,
                Err(BlockAncestryError::BlockIdMismatch { expected: key, actual: hash })
                    if key == expected && hash == actual
            ));
            // Reject the mismatched header before collecting or returning it.
            let expected_visited = if terminal {
                vec![first.compute_block_commitment()]
            } else {
                vec![]
            };
            assert_eq!(visited, expected_visited);
        }
    }

    #[test]
    fn test_storage_failure_preserved() {
        let start = make_block(1, 1, OLBlockId::from(Buf32::zero()), true, 1);
        // Fail the header read before ancestry can be inspected.
        let result = find_previous_epoch_terminal_block(
            start.header(),
            |_| Err(DbError::Busy),
            |_| panic!("a failed header read must not visit a block"),
        );
        assert!(matches!(result, Err(BlockAncestryError::Db(DbError::Busy))));
    }
}
