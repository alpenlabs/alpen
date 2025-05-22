use std::{
    collections::{HashMap, HashSet, VecDeque},
    num::NonZeroU8,
    sync::Arc,
    vec,
};

use bitcoin::Block;
use strata_btcio::reader::reader_task::ReaderCommand;
use strata_db::DbError;
use strata_primitives::l1::L1Block;
use strata_state::{
    client_state::{AnchorState, L1ClientState},
    l1::L1BlockId,
};
use strata_storage::{L1BlockManager, NodeStorage};
use thiserror::Error;
use tokio::sync::mpsc;
use tracing::error;

#[derive(Debug, Error)]
pub enum ChainTrackerError {
    #[error("other")]
    Other,
}

pub trait IndexableBlock {
    fn height(&self) -> u64;
    fn block_id(&self) -> L1BlockId;
    fn parent_id(&self) -> L1BlockId;
}

impl<T: IndexableBlock + ?Sized> IndexableBlock for &T {
    fn block_id(&self) -> L1BlockId {
        (*self).block_id()
    }
    fn parent_id(&self) -> L1BlockId {
        (*self).parent_id()
    }
    fn height(&self) -> u64 {
        (*self).height()
    }
}

impl IndexableBlock for L1Block {
    fn height(&self) -> u64 {
        self.height()
    }

    fn block_id(&self) -> L1BlockId {
        self.block_id()
    }

    fn parent_id(&self) -> L1BlockId {
        self.parent_id()
    }
}

#[derive(Debug)]
struct BlockEntry {
    block_id: L1BlockId,
    parent_id: L1BlockId,
    height: u64,
}

impl BlockEntry {
    fn new(height: u64, block_id: L1BlockId, parent_id: L1BlockId) -> Self {
        Self {
            block_id,
            parent_id,
            height,
        }
    }
}

impl IndexableBlock for BlockEntry {
    fn height(&self) -> u64 {
        self.height
    }
    fn block_id(&self) -> L1BlockId {
        self.block_id
    }
    fn parent_id(&self) -> L1BlockId {
        self.parent_id
    }
}

#[derive(Debug, Default)]
struct IndexedBlockTable {
    by_block_id: HashMap<L1BlockId, BlockEntry>,
    by_parent_id: HashMap<L1BlockId, Vec<L1BlockId>>,
    by_height: HashMap<u64, Vec<L1BlockId>>,
}

impl IndexedBlockTable {
    fn insert(&mut self, block: &impl IndexableBlock) {
        let height = block.height();
        let block_id = block.block_id();
        let parent_id = block.parent_id();

        self.by_block_id.insert(
            block_id,
            BlockEntry {
                height,
                parent_id,
                block_id,
            },
        );
        self.by_parent_id
            .entry(parent_id)
            .and_modify(|entry| entry.push(block_id))
            .or_insert(vec![block_id]);
        self.by_height
            .entry(height)
            .and_modify(|entry| entry.push(block_id))
            .or_insert(vec![block_id]);
    }

    fn remove(&mut self, block_id: &L1BlockId) -> Option<BlockEntry> {
        let block_entry = self.by_block_id.remove(block_id)?;

        self.by_parent_id
            .entry(block_entry.parent_id)
            .and_modify(|entries| entries.retain(|id| id != block_id));
        self.by_height
            .entry(block_entry.height)
            .and_modify(|entries| entries.retain(|id| id != block_id));

        Some(block_entry)
    }

    fn prune_to_height(&mut self, retain_min_height: u64) -> usize {
        let to_prune_blocks = self
            .by_height
            .iter()
            .filter_map(|(height, block_ids)| {
                if height < &retain_min_height {
                    Some(block_ids)
                } else {
                    None
                }
            })
            .flatten()
            .copied()
            .collect::<Vec<_>>();

        let count = to_prune_blocks.len();

        for block_id in to_prune_blocks {
            self.remove(&block_id);
        }

        count
    }
}

impl<I, B> From<I> for IndexedBlockTable
where
    I: IntoIterator<Item = B>,
    B: IndexableBlock,
{
    fn from(iterable: I) -> Self {
        let mut by_block_id = HashMap::new();
        let mut by_parent_id = HashMap::new();
        let mut by_height = HashMap::new();
        for block in iterable {
            let height = block.height();
            let block_id = block.block_id();
            let parent_id = block.parent_id();
            let block_data = BlockEntry {
                parent_id,
                height,
                block_id,
            };
            by_parent_id.insert(parent_id, vec![block_id]);
            by_height.insert(height, vec![block_id]);
            by_block_id.insert(block_id, block_data);
        }

        Self {
            by_block_id,
            by_parent_id,
            by_height,
        }
    }
}

struct CanonicalChain {
    base_height: u64,
    blocks: VecDeque<L1BlockId>,
}

impl CanonicalChain {
    fn new_from_base(block: &impl IndexableBlock) -> Self {
        Self {
            base_height: block.height(),
            blocks: vec![block.block_id()].into(),
        }
    }

    fn append(&mut self, block: &impl IndexableBlock) -> bool {
        if block.parent_id() != *self.tip() {
            false
        } else {
            self.blocks.push_back(block.block_id());
            true
        }
    }

    fn tip(&self) -> &L1BlockId {
        self.blocks.back().expect("cannot be empty")
    }

    fn tip_height(&self) -> u64 {
        self.base_height + self.blocks.len() as u64
    }
}

pub struct ChainTracker {
    canonical_chain: CanonicalChain,
    chain_tips: HashSet<L1BlockId>,
    chain: IndexedBlockTable,
    orphan_blocks: IndexedBlockTable,
}

pub enum AttachBlockResult {
    // Attached to parent
    Attached(Vec<L1BlockId>),
    // CanonicalChain,
    // SideChain,
    Orphan,
    Duplicate,
}

impl ChainTracker {
    pub fn attach_block(&mut self, block: &impl IndexableBlock) -> AttachBlockResult {
        if self.chain.by_block_id.contains_key(&block.block_id()) {
            return AttachBlockResult::Duplicate;
        }

        // if new block extends chain
        if self.chain.by_block_id.contains_key(&block.parent_id()) {
            let attached_blocks = self.attach_block_and_orphans(block);

            return AttachBlockResult::Attached(attached_blocks);
        }

        self.orphan_blocks.insert(block);
        AttachBlockResult::Orphan
    }

    fn add_block_to_chain(&mut self, block: &impl IndexableBlock) {
        // add to chain
        self.chain.insert(block);
        // remove parent from chain tip, if exists
        self.chain_tips.remove(&block.parent_id());
        // new block will always be a chain tip, either extend existing or start new sidechain
        self.chain_tips.insert(block.block_id());
    }

    fn attach_block_and_orphans(&mut self, block: &impl IndexableBlock) -> Vec<L1BlockId> {
        let mut queue = VecDeque::new();
        let mut attached_blocks = Vec::new();

        queue.push_back(block.block_id());

        self.add_block_to_chain(block);
        attached_blocks.push(block.block_id());

        if let Some(children) = self.orphan_blocks.by_parent_id.get(&block.block_id()) {
            queue.extend(children.iter());
        }

        while let Some(block_id) = queue.pop_front() {
            if let Some(block_data) = self.orphan_blocks.remove(&block_id) {
                self.add_block_to_chain(&block_data);
                attached_blocks.push(block_id);

                if let Some(children) = self.orphan_blocks.by_parent_id.get(&block_id) {
                    queue.extend(children.iter());
                }
            }
        }

        attached_blocks
    }

    pub fn prune(&mut self, retain_depth: NonZeroU8) -> usize {
        let tip_height = self.canonical_chain.tip_height();
        let Some(prune_to_height) = tip_height.checked_sub(retain_depth.get().into()) else {
            return 0;
        };

        let drain_count = prune_to_height.saturating_sub(self.canonical_chain.base_height) as usize;
        self.canonical_chain.blocks.drain(..drain_count);

        let chain_prune_count = self.chain.prune_to_height(prune_to_height);
        let orphan_prune_count = self.orphan_blocks.prune_to_height(prune_to_height);

        drain_count + chain_prune_count + orphan_prune_count
    }

    pub fn canonical_tip_height(&self) -> u64 {
        self.canonical_chain.tip_height()
    }

    pub fn canonical_tip(&self) -> &L1BlockId {
        self.canonical_chain.tip()
    }
}

pub fn init_chain_tracker() -> anyhow::Result<ChainTracker> {
    todo!()
}

fn expect_db_block(block_id: &L1BlockId, l1: &L1BlockManager) -> Result<L1Block, DbError> {
    l1.get_block(block_id)
        .transpose()
        .expect("csm: missing block")
}

pub fn csm_worker(
    mut rx: mpsc::Receiver<L1BlockId>,
    storage: Arc<NodeStorage>,
    command_tx: mpsc::Sender<ReaderCommand>,
) -> anyhow::Result<()> {
    let chain_ctx = make_chain_context(storage.clone());
    let mut chain_tracker = init_chain_tracker()?;

    while let Some(block_id) = rx.blocking_recv() {
        let block = match chain_ctx.expect_block(&block_id) {
            Ok(block) => block,
            Err(db_err) => {
                error!(%block_id, %db_err, "csm: failed to retrieve block from db");
                // TODO: retry
                continue;
            }
        };

        let attach_res = chain_tracker.attach_block(&block);
        match attach_res {
            AttachBlockResult::Duplicate => {
                continue;
            }
            AttachBlockResult::Orphan => {
                // fetch parent block
                let parent_id = block.parent_id();
                let _ = command_tx.blocking_send(ReaderCommand::FetchBlockById(parent_id));
                continue;
            }
            AttachBlockResult::Attached(attached_blocks) => {
                // build states for all new attached blocks
                for block_id in attached_blocks {
                    if let Err(err) = process_l1_block(&block_id, &chain_ctx) {
                        error!(%block_id, %err, "csm: failed to process block");
                    }
                }
                continue;
            }
        }
    }

    Ok(())
}

fn process_l1_block(block_id: &L1BlockId, ctx: &impl L1ChainContext) -> anyhow::Result<()> {
    let block = ctx.expect_block(block_id)?;

    let parent_id = block.parent_id();
    let prev_state = ctx.expect_client_state(&parent_id)?;

    let next_state = client_stf(&prev_state, &block, ctx)?;

    ctx.save_client_state(*block_id, next_state)?;

    Ok(())
}

trait L1ChainContext {
    fn expect_block(&self, block_id: &L1BlockId) -> Result<L1Block, DbError>;
    fn expect_client_state(&self, block_id: &L1BlockId) -> Result<L1ClientState, DbError>;

    fn save_client_state(&self, block_id: L1BlockId, state: L1ClientState) -> Result<(), DbError>;
}

fn make_chain_context(storage: Arc<NodeStorage>) -> impl L1ChainContext {
    DbL1ChainContext { storage }
}

struct DbL1ChainContext {
    storage: Arc<NodeStorage>,
}

impl L1ChainContext for DbL1ChainContext {
    fn expect_block(&self, block_id: &L1BlockId) -> Result<L1Block, DbError> {
        self.storage
            .l1()
            .get_block(block_id)
            .transpose()
            .expect("csm: missing block")
    }

    fn expect_client_state(&self, block_id: &L1BlockId) -> Result<L1ClientState, DbError> {
        self.storage
            .client_state()
            .get_l1_state_blocking(block_id)
            .transpose()
            .expect("csm: missing client state")
    }

    fn save_client_state(&self, block_id: L1BlockId, state: L1ClientState) -> Result<(), DbError> {
        self.storage
            .client_state()
            .put_l1_state_blocking(block_id, state)
    }
}

fn client_stf(
    prev_state: &L1ClientState,
    block: &L1Block,
    _ctx: &impl L1ChainContext,
) -> anyhow::Result<L1ClientState> {
    let anchor_state = asm_stf(prev_state.anchor_state(), block.inner())?;

    Ok(L1ClientState::new(block.block_id(), anchor_state))
}

fn asm_stf(_prev_state: &AnchorState, _block: &Block) -> anyhow::Result<AnchorState> {
    // placeholder
    todo!()
}
