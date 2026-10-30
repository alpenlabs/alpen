//! Block-history fixtures for ancestry-provider and consumer tests.
//!
//! [`BlockAncestryView`] builds OL block histories for testing ancestry-dependent behavior:
//!
//! - Build history from genesis with [`BlockAncestryView::append_block`] and
//!   [`BlockAncestryView::append_terminal_block`]. Append from an earlier block to fork.
//! - Get the result the code under test must return with
//!   [`BlockAncestryView::expected_epoch_ancestry`] and
//!   [`BlockAncestryView::expected_recent_history`]. These come from what was appended,
//!   not from walking the history.
//! - Promote a terminal block to the history base, as on a node started from a checkpoint,
//!   to test retained history. Recent-history results exclude the history base and older
//!   blocks, while epoch ancestry can still use its retained header.
//! - Break the history to test failures: drop a header or terminal body, append a malformed child
//!   with [`InvalidAncestry`], or substitute a header. Assert the resulting error directly;
//!   asking for an expected result that depends on a broken block panics.
//!
//! Use [`BlockAncestryView::new`] for in-memory tests and
//! [`BlockAncestryView::with_storage`] when the code under test reads an [`OLBlockManager`].
//! A storage-backed view applies each supported change to the manager too.

use std::collections::{HashMap, HashSet};
use std::iter::once;
use std::sync::Arc;

use strata_identifiers::{
    Buf32, Buf64, Epoch, EpochCommitment, OLBlockCommitment, OLBlockId, Slot,
};
use strata_ol_chain_types_v1::{
    BlockFlagsV1, OLBlockBodyV1, OLBlockHeaderV1, OLBlockV1, OLTxSegmentV1, SignedOLBlockHeaderV1,
};
use strata_storage::OLBlockManager;

use crate::EpochBlockAncestry;

#[derive(Clone, Debug)]
struct ExpectedEpochAncestry {
    previous_epoch: EpochCommitment,
    blocks: Vec<OLBlockCommitment>,
}

/// The append-only valid history behind a [`BlockAncestryView`].
///
/// Every non-genesis block is a valid child of a block already in this history. Visible-storage
/// deviations live in the owning view and never rewrite this record or its expected answers.
#[derive(Debug)]
struct BlockAncestryChain {
    blocks: HashMap<OLBlockId, OLBlockV1>,
    block_order: Vec<OLBlockId>,
    lineages: HashMap<OLBlockId, Vec<OLBlockCommitment>>,
    epoch_ancestry: HashMap<OLBlockId, ExpectedEpochAncestry>,
    genesis: OLBlockId,
}

impl BlockAncestryChain {
    /// Creates an append-only chain rooted at `genesis`.
    fn with_genesis(genesis: OLBlockV1) -> Self {
        assert_eq!(genesis.header().slot(), 0, "genesis must be at slot zero");
        assert_eq!(genesis.header().epoch(), 0, "genesis must be in epoch zero");
        assert!(genesis.header().is_terminal(), "genesis must be terminal");
        let genesis_id = genesis.header().compute_blkid();
        let genesis_commitment = genesis.header().compute_block_commitment();
        Self {
            blocks: HashMap::from([(genesis_id, genesis)]),
            block_order: vec![genesis_id],
            lineages: HashMap::from([(genesis_id, vec![genesis_commitment])]),
            epoch_ancestry: HashMap::new(),
            genesis: genesis_id,
        }
    }

    /// Returns the chain's genesis header.
    fn genesis(&self) -> &OLBlockHeaderV1 {
        self.header(self.genesis)
            .expect("test history always contains genesis")
    }

    /// Returns the valid-history header identified by `id`.
    fn header(&self, id: OLBlockId) -> Option<&OLBlockHeaderV1> {
        self.blocks.get(&id).map(OLBlockV1::header)
    }

    /// Iterates over valid-history blocks in append order.
    fn blocks(&self) -> impl Iterator<Item = &OLBlockV1> {
        self.block_order
            .iter()
            .map(|id| self.blocks.get(id).expect("ordered block must exist"))
    }

    /// Returns whether the valid history contains the block identified by `id`.
    fn contains_block(&self, id: OLBlockId) -> bool {
        self.blocks.contains_key(&id)
    }

    /// Records a valid child and its independently derived ancestry expectations.
    fn record_block(&mut self, block: OLBlockV1) -> OLBlockHeaderV1 {
        let header = block.header().clone();
        let id = header.compute_blkid();
        let parent_id = *header.parent_blkid();
        let parent = self
            .header(parent_id)
            .expect("valid child must have a valid-history parent");

        let mut lineage = self
            .lineages
            .get(&parent_id)
            .cloned()
            .expect("valid parent must have a recorded lineage");
        lineage.push(header.compute_block_commitment());

        let expected = if parent.is_terminal() {
            ExpectedEpochAncestry {
                previous_epoch: EpochCommitment::from_terminal(
                    parent.epoch(),
                    parent.compute_block_commitment(),
                ),
                blocks: vec![header.compute_block_commitment()],
            }
        } else {
            let mut expected = self
                .epoch_ancestry
                .get(&parent_id)
                .cloned()
                .expect("valid nonterminal parent must have epoch ancestry");
            expected.blocks.push(header.compute_block_commitment());
            expected
        };

        assert!(self.blocks.insert(id, block).is_none(), "block must be new");
        self.block_order.push(id);
        self.lineages.insert(id, lineage);
        self.epoch_ancestry.insert(id, expected);
        header
    }
}

/// A deliberately invalid relationship between a fixture block and its parent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvalidAncestry {
    /// Places the child at a slot other than the slot immediately following its parent.
    NonAdjacentSlot(Slot),
    /// Assigns epoch metadata inconsistent with the epoch derived from the parent.
    EpochMetadata {
        /// Epoch assigned to the child.
        epoch: Epoch,
        /// Whether the child is marked as the assigned epoch's terminal block.
        terminal: bool,
    },
}

/// A coherent history together with the storage deviations visible to a test.
#[expect(
    missing_debug_implementations,
    reason = "storage managers do not implement Debug"
)]
pub struct BlockAncestryView {
    chain: BlockAncestryChain,
    ol_block: Option<Arc<OLBlockManager>>,
    next_identity: u64,
    history_base: Option<EpochCommitment>,
    dropped_bodies: HashSet<OLBlockId>,
    dropped_headers: HashSet<OLBlockId>,
    substituted_headers: HashMap<OLBlockId, OLBlockId>,
    overlays: HashMap<OLBlockId, OLBlockV1>,
}

impl BlockAncestryView {
    /// Creates a view containing only a terminal epoch-zero genesis block.
    #[expect(
        clippy::new_without_default,
        reason = "a synthetic block-history fixture has no domain default"
    )]
    pub fn new() -> Self {
        Self::from_genesis(make_genesis_block(), None)
    }

    /// Creates a view that also writes genesis and every later change to `ol_block`.
    ///
    /// `ol_block` must hold no blocks and no history base. Create the view before writing
    /// anything that refers to genesis, and take genesis from [`Self::genesis`]. The view does
    /// not model the canonical index: publish it after any deviations, which can truncate it.
    ///
    /// # Panics
    ///
    /// Panics when `ol_block` holds a block or a history base, or a storage write fails.
    pub fn with_storage(ol_block: Arc<OLBlockManager>) -> Self {
        Self::from_genesis(make_genesis_block(), Some(ol_block))
    }

    /// Returns the genesis header.
    pub fn genesis(&self) -> &OLBlockHeaderV1 {
        self.chain.genesis()
    }

    /// Returns the header visible after applying this view's storage deviations.
    pub fn visible_header(&self, id: OLBlockId) -> Option<OLBlockHeaderV1> {
        if self.dropped_headers.contains(&id) {
            return None;
        }
        if let Some(header) = self.chain.header(id) {
            if self.header_is_pruned(id, header) {
                return None;
            }
            if let Some(actual) = self.substituted_headers.get(&id) {
                return self.chain.header(*actual).cloned();
            }
            return Some(header.clone());
        }
        if let Some(block) = self.overlays.get(&id) {
            return (!self.header_is_pruned(id, block.header())).then(|| block.header().clone());
        }
        None
    }

    /// Appends a valid nonterminal child of `parent`.
    ///
    /// # Panics
    ///
    /// Panics when `parent` is not a visible valid-history block, or a storage write fails.
    pub fn append_block(&mut self, parent: &OLBlockHeaderV1) -> OLBlockHeaderV1 {
        let block = self.build_child_block(parent, false);
        self.store_block(&block);
        self.chain.record_block(block)
    }

    /// Appends a valid terminal child of `parent`.
    ///
    /// # Panics
    ///
    /// Panics when `parent` is not a visible valid-history block, or a storage write fails.
    pub fn append_terminal_block(&mut self, parent: &OLBlockHeaderV1) -> OLBlockHeaderV1 {
        let block = self.build_child_block(parent, true);
        self.store_block(&block);
        self.chain.record_block(block)
    }

    /// Appends a deliberately malformed child of a visible valid-history or overlay block.
    ///
    /// A storage-backed view writes the malformed full block before exposing it through the view.
    /// The block never enters the valid history or receives successful ancestry expectations.
    ///
    /// # Panics
    ///
    /// Panics when `parent` is not visible, the requested relationship is valid, the child's slot
    /// is at or below the history base, the new block ID collides with an existing valid-history
    /// or overlay block, or a storage write fails.
    pub fn append_invalid_child(
        &mut self,
        parent: &OLBlockHeaderV1,
        invalidity: InvalidAncestry,
    ) -> OLBlockHeaderV1 {
        let block = self.build_invalid_child_block(parent, invalidity);
        self.store_block(&block);
        self.record_invalid_child_block(block)
    }

    /// Makes a keyed read for `requested` return the valid-history header identified by `actual`.
    ///
    /// This models deliberately corrupt in-memory storage for block-ID validation tests.
    ///
    /// # Panics
    ///
    /// Panics for a storage-backed view, because [`OLBlockManager`] rejects mismatched keyed
    /// headers. Also panics when either ID is not a distinct valid-history block or conflicts with
    /// another deviation.
    pub fn substitute_header(&mut self, requested: OLBlockId, actual: OLBlockId) {
        assert!(
            self.ol_block.is_none(),
            "keyed header substitution is available only in memory"
        );
        assert_ne!(requested, actual, "substitution must change the header");
        assert!(
            self.chain.contains_block(requested),
            "requested header must belong to valid history"
        );
        assert!(
            self.chain.contains_block(actual),
            "substitute header must belong to valid history"
        );
        assert!(
            !self.dropped_headers.contains(&requested) && !self.dropped_headers.contains(&actual),
            "a header cannot be both dropped and substituted"
        );
        assert!(
            !self.substitution_mentions(requested) && !self.substitution_mentions(actual),
            "substitutions cannot overlap"
        );
        self.substituted_headers.insert(requested, actual);
    }

    /// Promotes a valid-history terminal to the history base.
    ///
    /// Promoting the same terminal again is idempotent.
    ///
    /// # Panics
    ///
    /// Panics when the header is not a visible valid-history terminal, a different history base
    /// was already promoted, a malformed overlay sits at or below the new history base, an older
    /// terminal header was retained separately and cannot be pruned, or a storage write fails.
    pub fn promote_history_anchor(&mut self, terminal: &OLBlockHeaderV1) -> EpochCommitment {
        let base = self.validate_history_base(terminal);
        if self.history_base == Some(base) {
            return base;
        }
        if let Some(ol_block) = self.ol_block.clone() {
            self.store_history_anchor(&ol_block, terminal, base);
        }
        self.history_base = Some(base);
        base
    }

    /// Removes a valid-history header and its body from the visible view.
    ///
    /// # Panics
    ///
    /// Panics when the header is not visible, conflicts with another deviation, is the history
    /// base, is stored separately as a terminal header, or a storage write fails.
    pub fn drop_header(&mut self, id: OLBlockId) {
        self.assert_header_can_be_dropped(id);
        if let Some(ol_block) = &self.ol_block {
            assert!(
                ol_block
                    .get_terminal_header_blocking(id)
                    .expect("test: inspect retained terminal header")
                    .is_none(),
                "cannot drop a separately retained terminal header"
            );
            let deleted = ol_block
                .del_block_data_blocking(id)
                .expect("test: delete block header and body");
            assert!(deleted, "visible valid-history full block must exist");
        }
        self.dropped_headers.insert(id);
    }

    /// Removes a terminal block's body while retaining its header in the visible view.
    ///
    /// # Panics
    ///
    /// Panics when `id` is not a visible valid-history terminal with an available body, conflicts
    /// with another deviation, or a storage write fails.
    pub fn drop_body(&mut self, id: OLBlockId) {
        let header = self
            .chain
            .header(id)
            .expect("body drop must name a valid-history block")
            .clone();
        assert_eq!(
            self.visible_header(id).as_ref(),
            Some(&header),
            "body drop requires an unsubstituted visible header"
        );
        assert!(
            !self.substitution_mentions(id),
            "a header cannot be both body-dropped and substituted"
        );
        assert!(header.is_terminal(), "only terminal bodies can be dropped");
        assert!(!self.body_is_pruned(&header), "block body must be visible");
        assert!(
            !self.dropped_bodies.contains(&id),
            "block body cannot be dropped twice"
        );

        if let Some(ol_block) = &self.ol_block {
            ol_block
                .put_terminal_header_blocking(id, header)
                .expect("test: retain terminal header before dropping body");
            let deleted = ol_block
                .del_block_data_blocking(id)
                .expect("test: delete terminal block body");
            assert!(deleted, "visible terminal block body must exist");
        }
        self.dropped_bodies.insert(id);
    }

    /// Returns the epoch ancestry the provider must resolve through `start`.
    ///
    /// # Panics
    ///
    /// Panics when `start` is not a valid non-genesis history block or a view deviation touches a
    /// header needed for the answer.
    pub fn expected_epoch_ancestry(&self, start: &OLBlockHeaderV1) -> EpochBlockAncestry {
        let expected = self
            .chain
            .epoch_ancestry
            .get(&start.compute_blkid())
            .expect("expected ancestry requires a valid non-genesis history block");
        let dependencies = expected
            .blocks
            .iter()
            .map(|block| *block.blkid())
            .chain(once(*expected.previous_epoch.last_blkid()));
        self.assert_headers_reliable(dependencies);
        EpochBlockAncestry::from_parts(expected.previous_epoch, expected.blocks.clone())
    }

    /// Returns the recent-history window the provider must resolve through `start`.
    ///
    /// # Panics
    ///
    /// Panics when `start` is not a valid history block, its branch conflicts with the history
    /// base, or a view deviation touches a header in the returned window.
    pub fn expected_recent_history(
        &self,
        start: &OLBlockHeaderV1,
        max_blocks: usize,
    ) -> Vec<OLBlockCommitment> {
        let lineage = self
            .chain
            .lineages
            .get(&start.compute_blkid())
            .expect("expected history requires a valid history block");
        if max_blocks == 0
            || self
                .history_base
                .is_some_and(|base| start.slot() <= base.last_slot())
        {
            return Vec::new();
        }

        let retained = if let Some(base) = self.history_base {
            let boundary = lineage
                .iter()
                .position(|block| block.slot() == base.last_slot())
                .expect("start lineage must reach the history base slot");
            assert_eq!(
                lineage[boundary],
                base.to_block_commitment(),
                "start branch must contain the history base"
            );
            &lineage[boundary + 1..]
        } else {
            lineage.as_slice()
        };
        let selected = &retained[retained.len().saturating_sub(max_blocks)..];
        self.assert_headers_reliable(selected.iter().map(|block| *block.blkid()));
        selected.to_vec()
    }

    /// Builds a view around `genesis` and optionally persists it through `ol_block`.
    fn from_genesis(genesis: OLBlockV1, ol_block: Option<Arc<OLBlockManager>>) -> Self {
        if let Some(ol_block) = &ol_block {
            Self::assert_manager_has_no_history(ol_block);
        }
        let stored_genesis = ol_block.as_ref().map(|_| genesis.clone());
        let chain = BlockAncestryChain::with_genesis(genesis);
        if let (Some(ol_block), Some(genesis)) = (&ol_block, stored_genesis) {
            ol_block
                .put_block_data_blocking(genesis)
                .expect("test: store genesis");
        }
        Self {
            chain,
            ol_block,
            next_identity: 1,
            history_base: None,
            dropped_bodies: HashSet::new(),
            dropped_headers: HashSet::new(),
            substituted_headers: HashMap::new(),
            overlays: HashMap::new(),
        }
    }

    /// Panics unless `ol_block` holds no blocks and no history base.
    ///
    /// Only these are ancestry state; the canonical index and other manager state belong to the
    /// consumer.
    fn assert_manager_has_no_history(ol_block: &OLBlockManager) {
        assert!(
            ol_block
                .get_highest_block_slot_blocking()
                .expect("test: query highest stored slot")
                .is_none(),
            "test block manager must contain no full blocks"
        );
        assert!(
            ol_block
                .get_history_base_blocking()
                .expect("test: query history base")
                .is_none(),
            "test block manager must have no history base"
        );
    }

    /// Returns the next fixture identity and advances the counter.
    fn take_identity(&mut self) -> u64 {
        let identity = self.next_identity;
        self.next_identity = self
            .next_identity
            .checked_add(1)
            .expect("test block identity must fit");
        identity
    }

    /// Builds the next valid child and reserves its fixture identity without recording or
    /// storing it.
    fn build_child_block(&mut self, parent: &OLBlockHeaderV1, terminal: bool) -> OLBlockV1 {
        let parent_id = parent.compute_blkid();
        assert!(
            self.chain.contains_block(parent_id),
            "valid parent must belong to valid history"
        );
        assert_eq!(
            self.visible_header(parent_id).as_ref(),
            Some(parent),
            "valid parent must be visible without substitution"
        );
        let identity = self.take_identity();
        let block = make_block(
            derive_child_slot(parent),
            derive_child_epoch(parent),
            parent_id,
            terminal,
            identity,
        );
        assert!(
            !self.chain.contains_block(block.header().compute_blkid()),
            "child block must be distinct"
        );
        block
    }

    /// Builds a malformed child and reserves its fixture identity without recording or storing it.
    fn build_invalid_child_block(
        &mut self,
        parent: &OLBlockHeaderV1,
        invalidity: InvalidAncestry,
    ) -> OLBlockV1 {
        let parent_id = parent.compute_blkid();
        assert!(
            self.chain.contains_block(parent_id) || self.overlays.contains_key(&parent_id),
            "invalid child parent must belong to the view"
        );
        assert_eq!(
            self.visible_header(parent_id).as_ref(),
            Some(parent),
            "invalid child parent must be visible"
        );
        let (slot, epoch, terminal) = match invalidity {
            InvalidAncestry::NonAdjacentSlot(slot) => {
                assert_ne!(
                    slot,
                    derive_child_slot(parent),
                    "child slot must be non-adjacent"
                );
                (slot, derive_child_epoch(parent), false)
            }
            InvalidAncestry::EpochMetadata { epoch, terminal } => {
                assert_ne!(
                    epoch,
                    derive_child_epoch(parent),
                    "child epoch must be inconsistent with its parent"
                );
                (derive_child_slot(parent), epoch, terminal)
            }
        };
        if let Some(base) = self.history_base {
            assert!(
                slot > base.last_slot(),
                "malformed overlay must be above the history base"
            );
        }
        let identity = self.take_identity();
        let block = make_block(slot, epoch, parent_id, terminal, identity);
        let id = block.header().compute_blkid();
        assert!(
            !self.chain.contains_block(id) && !self.overlays.contains_key(&id),
            "overlay block ID cannot collide"
        );
        block
    }

    /// Stores `block` when this view has storage backing.
    fn store_block(&self, block: &OLBlockV1) {
        if let Some(ol_block) = &self.ol_block {
            ol_block
                .put_block_data_blocking(block.clone())
                .expect("test: store appended block");
        }
    }

    /// Records a malformed overlay.
    fn record_invalid_child_block(&mut self, block: OLBlockV1) -> OLBlockHeaderV1 {
        let header = block.header().clone();
        let id = header.compute_blkid();
        assert!(
            self.overlays.insert(id, block).is_none(),
            "overlay block must be new"
        );
        header
    }

    /// Verifies that `terminal` can become the history base and returns its epoch commitment.
    fn validate_history_base(&self, terminal: &OLBlockHeaderV1) -> EpochCommitment {
        let id = terminal.compute_blkid();
        assert!(
            self.chain.contains_block(id),
            "history base must belong to valid history"
        );
        assert_eq!(
            self.visible_header(id).as_ref(),
            Some(terminal),
            "history base header must be visible"
        );
        assert!(terminal.is_terminal(), "history base must be terminal");
        let base =
            EpochCommitment::from_terminal(terminal.epoch(), terminal.compute_block_commitment());
        if let Some(existing) = self.history_base {
            assert_eq!(existing, base, "history base cannot change");
        }
        assert!(
            self.overlays
                .values()
                .all(|block| block.header().slot() > base.last_slot()),
            "malformed overlay must be above the history base"
        );
        assert!(
            self.dropped_bodies.iter().all(|id| {
                self.chain
                    .header(*id)
                    .expect("dropped body must belong to valid history")
                    .slot()
                    >= base.last_slot()
            }),
            "cannot prune an older retained terminal header"
        );
        base
    }

    /// Persists the history base and prunes its unavailable full-block prefix.
    fn store_history_anchor(
        &self,
        ol_block: &OLBlockManager,
        terminal: &OLBlockHeaderV1,
        base: EpochCommitment,
    ) {
        assert!(
            !self.chain.blocks().any(|block| {
                block.header().slot() < base.last_slot()
                    && ol_block
                        .get_terminal_header_blocking(block.header().compute_blkid())
                        .expect("test: inspect retained terminal header")
                        .is_some()
            }),
            "cannot prune an older retained terminal header"
        );

        ol_block
            .put_terminal_header_blocking(terminal.compute_blkid(), terminal.clone())
            .expect("test: retain history base header");
        let pruned_ids: Vec<_> = self
            .chain
            .blocks()
            .filter(|block| block.header().slot() <= base.last_slot())
            .map(|block| block.header().compute_blkid())
            .collect();
        for id in pruned_ids {
            let deleted = ol_block
                .del_block_data_blocking(id)
                .expect("test: prune retained history block");
            assert!(
                deleted || self.dropped_bodies.contains(&id) || self.dropped_headers.contains(&id),
                "visible valid-history full block must exist before pruning"
            );
        }
        ol_block
            .promote_to_history_anchor_blocking(base)
            .expect("test: promote history anchor");
    }

    /// Panics unless `id` names a visible, removable valid-history header.
    fn assert_header_can_be_dropped(&self, id: OLBlockId) {
        let header = self
            .chain
            .header(id)
            .expect("dropped header must belong to valid history");
        assert!(
            !self.header_is_pruned(id, header),
            "dropped header must be visible"
        );
        assert!(
            !self.dropped_headers.contains(&id),
            "header cannot be dropped twice"
        );
        assert!(
            !self.dropped_bodies.contains(&id),
            "a retained header cannot be dropped after its body"
        );
        assert!(
            !self.substitution_mentions(id),
            "a header cannot be both dropped and substituted"
        );
        assert!(
            self.history_base
                .is_none_or(|base| id != *base.last_blkid()),
            "history base header cannot be dropped"
        );
    }

    /// Returns whether the history base hides `header` under `id`.
    fn header_is_pruned(&self, id: OLBlockId, header: &OLBlockHeaderV1) -> bool {
        self.history_base.is_some_and(|base| {
            header.slot() < base.last_slot()
                || (header.slot() == base.last_slot() && id != *base.last_blkid())
        })
    }

    /// Returns whether the history base makes `header`'s body unavailable.
    fn body_is_pruned(&self, header: &OLBlockHeaderV1) -> bool {
        self.history_base
            .is_some_and(|base| header.slot() <= base.last_slot())
    }

    /// Returns whether `id` participates in a keyed-header substitution.
    fn substitution_mentions(&self, id: OLBlockId) -> bool {
        self.substituted_headers.contains_key(&id)
            || self
                .substituted_headers
                .values()
                .any(|actual| *actual == id)
    }

    /// Panics when a deviation makes any expected-answer header unreliable.
    fn assert_headers_reliable(&self, ids: impl IntoIterator<Item = OLBlockId>) {
        for id in ids {
            let header = self
                .chain
                .header(id)
                .expect("expected-answer dependency must belong to valid history");
            assert!(
                !self.dropped_headers.contains(&id)
                    && !self.substitution_mentions(id)
                    && !self.header_is_pruned(id, header),
                "view deviation touches an expected-answer header"
            );
        }
    }
}

/// Derives the slot immediately following `parent`.
fn derive_child_slot(parent: &OLBlockHeaderV1) -> Slot {
    parent
        .slot()
        .checked_add(1)
        .expect("test child slot must fit")
}

/// Derives the epoch of a valid child of `parent`.
fn derive_child_epoch(parent: &OLBlockHeaderV1) -> Epoch {
    parent
        .epoch()
        .checked_add(Epoch::from(parent.is_terminal()))
        .expect("test child epoch must fit")
}

/// Builds the synthetic fixture genesis with a null parent.
fn make_genesis_block() -> OLBlockV1 {
    make_block(0, 0, OLBlockId::null(), true, 0)
}

/// Builds a deterministic fixture block from explicit header metadata.
fn make_block(
    slot: Slot,
    epoch: Epoch,
    parent: OLBlockId,
    terminal: bool,
    identity: u64,
) -> OLBlockV1 {
    let body = OLBlockBodyV1::new_common(
        OLTxSegmentV1::new(vec![]).expect("empty transaction segment is valid"),
    );
    let mut flags = BlockFlagsV1::zero();
    flags.set_is_terminal(terminal);
    let timestamp = 1_000_u64
        .checked_add(slot)
        .expect("test block timestamp must fit");
    let mut identity_bytes = [0_u8; 32];
    identity_bytes[..8].copy_from_slice(&identity.to_le_bytes());
    let header = OLBlockHeaderV1::new(
        timestamp,
        flags,
        slot,
        epoch,
        parent,
        body.compute_hash_commitment(),
        Buf32::from(identity_bytes),
        Buf32::zero(),
    );
    OLBlockV1::new(SignedOLBlockHeaderV1::new(header, Buf64::zero()), body)
}

#[cfg(test)]
mod tests {
    use super::{BlockAncestryView, InvalidAncestry};

    #[test]
    #[should_panic(expected = "malformed overlay must be above the history base")]
    fn test_history_base_promotion_rejects_overlay_at_boundary() {
        let mut view = BlockAncestryView::new();
        let genesis = view.genesis().clone();
        let _overlay = view.append_invalid_child(&genesis, InvalidAncestry::NonAdjacentSlot(0));
        let base = view.append_terminal_block(&genesis);

        // Promotion would prune the overlay from the view but leave it in manager-backed storage.
        view.promote_history_anchor(&base);
    }

    #[test]
    #[should_panic(expected = "malformed overlay must be above the history base")]
    fn test_malformed_overlay_stays_above_history_base() {
        let mut view = BlockAncestryView::new();
        let genesis = view.genesis().clone();
        let base = view.append_terminal_block(&genesis);
        view.promote_history_anchor(&base);

        // A lower overlay would remain stored while the retained view hid it.
        let _overlay =
            view.append_invalid_child(&base, InvalidAncestry::NonAdjacentSlot(base.slot()));
    }

    #[test]
    fn test_unrelated_header_drop_keeps_epoch_expectation_available() {
        let mut view = BlockAncestryView::new();
        let genesis = view.genesis().clone();
        let unrelated = view.append_block(&genesis);
        let previous = view.append_terminal_block(&genesis);
        let start = view.append_terminal_block(&previous);

        view.drop_header(unrelated.compute_blkid());

        assert_eq!(
            view.expected_epoch_ancestry(&start).blocks(),
            &[start.compute_block_commitment()]
        );
    }

    #[test]
    #[should_panic(expected = "view deviation touches an expected-answer header")]
    fn test_required_header_drop_invalidates_epoch_expectation() {
        let mut view = BlockAncestryView::new();
        let genesis = view.genesis().clone();
        let previous = view.append_terminal_block(&genesis);
        let start = view.append_terminal_block(&previous);

        view.drop_header(previous.compute_blkid());
        let _ = view.expected_epoch_ancestry(&start);
    }

    #[test]
    fn test_terminal_body_drop_keeps_header_ancestry_available() {
        let mut view = BlockAncestryView::new();
        let genesis = view.genesis().clone();
        let previous = view.append_terminal_block(&genesis);
        let start = view.append_terminal_block(&previous);

        view.drop_body(previous.compute_blkid());

        assert_eq!(
            view.visible_header(previous.compute_blkid()),
            Some(previous.clone())
        );
        assert_eq!(
            view.expected_epoch_ancestry(&start)
                .previous_epoch()
                .to_block_commitment(),
            previous.compute_block_commitment()
        );
    }

    #[test]
    #[should_panic(expected = "only terminal bodies can be dropped")]
    fn test_nonterminal_body_cannot_be_dropped_alone() {
        let mut view = BlockAncestryView::new();
        let genesis = view.genesis().clone();
        let nonterminal = view.append_block(&genesis);

        view.drop_body(nonterminal.compute_blkid());
    }

    #[test]
    #[should_panic(expected = "cannot prune an older retained terminal header")]
    fn test_history_base_cannot_advance_past_dropped_terminal_body() {
        let mut view = BlockAncestryView::new();
        let genesis = view.genesis().clone();
        let retained = view.append_terminal_block(&genesis);
        let later = view.append_terminal_block(&retained);
        view.drop_body(retained.compute_blkid());

        view.promote_history_anchor(&later);
    }

    #[test]
    fn test_recent_history_expectation_stops_before_history_base() {
        let mut view = BlockAncestryView::new();
        let genesis = view.genesis().clone();
        let base = view.append_terminal_block(&genesis);
        let first = view.append_block(&base);
        let tip = view.append_terminal_block(&first);
        view.promote_history_anchor(&base);

        assert_eq!(
            view.expected_recent_history(&tip, usize::MAX),
            vec![
                first.compute_block_commitment(),
                tip.compute_block_commitment()
            ]
        );
    }
}
