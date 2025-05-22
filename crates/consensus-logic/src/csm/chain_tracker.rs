use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
    vec,
};

use bitcoin::{block::Header, Block};
use strata_btcio::reader::reader_task::ReaderCommand;
use strata_db::DbError;
use strata_primitives::l1::L1Block;
use strata_state::{
    client_state::{AnchorState, L1ClientState},
    l1::L1BlockId,
};
use strata_storage::NodeStorage;
use thiserror::Error;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

const MAX_RETRIES: u8 = 10;

// TODO: use correct errors instead of anyhow
#[derive(Debug, Error)]
pub enum ChainTrackerError {
    #[error("other")]
    Other,
}

#[derive(Debug, Clone, Copy)]
pub struct L1Header {
    height: u64,
    block_id: L1BlockId,
    accumulated_pow: U256,
    inner: Header,
}

impl L1Header {
    pub fn new(height: u64, accumulated_pow: U256, header: Header) -> Self {
        Self {
            height,
            block_id: header.block_hash().into(),
            accumulated_pow,
            inner: header,
        }
    }

    pub fn block_id(&self) -> L1BlockId {
        self.block_id
    }

    pub fn parent_id(&self) -> L1BlockId {
        self.inner.prev_blockhash.into()
    }

    pub fn height(&self) -> u64 {
        self.height
    }

    pub fn inner(&self) -> &Header {
        &self.inner
    }

    pub fn from_block(block: &L1Block, accumulated_pow: U256) -> Self {
        Self {
            height: block.height(),
            block_id: block.block_id(),
            accumulated_pow,
            inner: block.inner().header,
        }
    }
}

struct L1BlockWithPow {
    inner: L1Block,
    accumulated_pow: U256,
}

impl L1BlockWithPow {
    pub fn from_block(block: L1Block, accumulated_pow: U256) -> Self {
        Self {
            inner: block,
            accumulated_pow,
        }
    }

    pub fn inner(&self) -> &L1Block {
        &self.inner
    }

    pub fn accumulated_pow(&self) -> &U256 {
        &self.accumulated_pow
    }
}

#[derive(Debug, Default)]
struct IndexedBlockTable {
    by_block_id: HashMap<L1BlockId, L1Header>,
    by_parent_id: HashMap<L1BlockId, Vec<L1BlockId>>,
    by_height: HashMap<u64, Vec<L1BlockId>>,
}

impl IndexedBlockTable {
    fn insert(&mut self, block: L1Header) {
        let height = block.height();
        let block_id = block.block_id();
        let parent_id = block.parent_id();

        self.by_block_id.insert(block_id, block);
        self.by_parent_id
            .entry(parent_id)
            .and_modify(|entry| entry.push(block_id))
            .or_insert(vec![block_id]);
        self.by_height
            .entry(height)
            .and_modify(|entry| entry.push(block_id))
            .or_insert(vec![block_id]);
    }

    fn remove(&mut self, block_id: &L1BlockId) -> Option<L1Header> {
        let block = self.by_block_id.remove(block_id)?;

        self.by_parent_id
            .entry(block.parent_id())
            .and_modify(|entries| entries.retain(|id| id != block_id));
        self.by_height
            .entry(block.height())
            .and_modify(|entries| entries.retain(|id| id != block_id));

        Some(block)
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

pub enum AttachBlockResult {
    Attachable,
    Orphan,
    Duplicate,
    BelowSafeHeight,
}

pub struct ChainTracker {
    // currently tracked tip blocks
    chain_tips: HashSet<L1BlockId>,
    // blocks > safe_height
    chain: IndexedBlockTable,
    // height below which we dont track for reorgs
    safe_height: u64,
}

impl ChainTracker {
    pub fn test_attach_block(&self, block: &L1Block) -> AttachBlockResult {
        if block.height() <= self.safe_height {
            return AttachBlockResult::BelowSafeHeight;
        }

        if self.chain.by_block_id.contains_key(&block.block_id()) {
            return AttachBlockResult::Duplicate;
        }

        // if new block extends chain
        if self.chain.by_block_id.contains_key(&block.parent_id()) {
            return AttachBlockResult::Attachable;
        }

        AttachBlockResult::Orphan
    }

    pub fn attach_block_unchecked(&mut self, block: L1Header) {
        self.chain_tips.remove(&block.parent_id());
        self.chain_tips.insert(block.block_id());
        self.chain.insert(block);
    }

    pub fn prune(&mut self, min_height: u64) -> usize {
        self.chain.prune_to_height(min_height)
    }

    pub fn best_block(&self) -> &L1Header {
        let best_id = self
            .chain_tips
            .iter()
            .max_by(|a, b| {
                self.chain.by_block_id[*a]
                    .accumulated_pow
                    .cmp(&self.chain.by_block_id[*b].accumulated_pow)
            })
            .unwrap();
        &self.chain.by_block_id[best_id]
    }
}

pub fn init_chain_tracker(_storage: &NodeStorage) -> anyhow::Result<ChainTracker> {
    todo!()
}

struct WorkItem {
    block_id: L1BlockId,
    retry_count: u8,
}

impl WorkItem {
    fn new(block_id: L1BlockId) -> Self {
        Self {
            block_id,
            retry_count: 0,
        }
    }

    fn retry(mut self) -> Self {
        self.retry_count += 1;
        self
    }
}

pub fn csm_worker(
    mut block_rx: mpsc::Receiver<L1BlockId>,
    command_tx: mpsc::Sender<ReaderCommand>,
    storage: Arc<NodeStorage>,
) -> anyhow::Result<()> {
    let chain_ctx = make_chain_context(storage.clone());
    let mut chain_tracker = init_chain_tracker(storage.as_ref())?;
    let mut orphan_tracker = IndexedBlockTable::default();
    let mut work_queue = VecDeque::new();

    loop {
        while let Ok(new_block_id) = block_rx.try_recv() {
            let block = match chain_ctx.get_block(&new_block_id) {
                Ok(Some(block)) => block,
                Ok(None) => {
                    // TODO: retry
                    error!(%new_block_id, "csm: missing block");
                    continue;
                }
                Err(db_err) => {
                    error!(%db_err, "csm: failed to retrieve block from db");
                    continue;
                }
            };

            match chain_tracker.test_attach_block(&block) {
                AttachBlockResult::BelowSafeHeight => {
                    warn!(block_id = %block.block_id(), "csm: block below safe height");
                    continue;
                }
                AttachBlockResult::Duplicate => {
                    warn!(block_id = %block.block_id(), "csm: duplicate block");
                    continue;
                }
                AttachBlockResult::Orphan => {
                    info!(block_id = %block.block_id(), parent_id = %block.parent_id(), "csm: orphan block");
                    // try to fetch parent block
                    let _ =
                        command_tx.blocking_send(ReaderCommand::FetchBlockById(block.parent_id()));

                    orphan_tracker.insert(L1Header::from_block(&block, U256::zero()));
                }
                AttachBlockResult::Attachable => {
                    work_queue.push_back(WorkItem::new(block.block_id()));
                }
            }
        }

        if let Some(work) = work_queue.pop_front() {
            let block_id = work.block_id;
            let block = chain_ctx.expect_block(&block_id);

            match process_l1_block(&block, &chain_ctx) {
                Ok(ProcessBlockResult::Valid(accumulated_pow)) => {
                    // add to chain tracker
                    chain_tracker
                        .attach_block_unchecked(L1Header::from_block(&block, accumulated_pow));

                    // check if any orphan blocks can be attached to this block
                    if let Some(children) = orphan_tracker.by_parent_id.get(&block_id).cloned() {
                        for child in children.iter().rev() {
                            // add them to queue
                            orphan_tracker.remove(child);
                            work_queue.push_front(WorkItem::new(*child));
                        }
                    }
                }
                Ok(ProcessBlockResult::Invalid) => {}
                Err(err) => {
                    // TODO: check for non recoverable errors
                    warn!(%block_id, retry = %work.retry_count, %err, "csm: failed to process block");

                    if work.retry_count < MAX_RETRIES {
                        work_queue.push_back(work.retry());
                    } else {
                        error!(%block_id, "csm: max retries reached")
                    }
                }
            };
        }
    }
}

enum ProcessBlockResult {
    Valid(U256),
    Invalid,
}

fn process_l1_block(
    block: &L1Block,
    ctx: &impl L1ChainContext,
) -> anyhow::Result<ProcessBlockResult> {
    let block_id = block.block_id();
    let parent_id = block.parent_id();

    let prev_state = ctx.expect_client_state(&parent_id);

    match client_stf(&prev_state, block, ctx)? {
        BlockStatus::Valid(next_state) => {
            // calculate accumulated pow for this block
            let parent_accumulated_pow = ctx.expect_block_pow(&parent_id);
            let block_pow = U256::from_be_bytes(block.inner().header.work().to_be_bytes());
            let accumulated_pow = parent_accumulated_pow.saturating_add(block_pow);

            // update db
            ctx.save_client_state(block_id, next_state)?;
            ctx.mark_block_valid(&block_id, block.height(), accumulated_pow)?;

            Ok(ProcessBlockResult::Valid(accumulated_pow))
        }
        BlockStatus::Invalid => {
            // remove invalid block from db
            ctx.remove_invalid_block(&block_id)?;

            Ok(ProcessBlockResult::Invalid)
        }
    }
}

trait L1ChainContext {
    fn get_block(&self, block_id: &L1BlockId) -> Result<Option<L1Block>, DbError>;
    fn get_block_with_pow(&self, block_id: &L1BlockId) -> Result<Option<L1BlockWithPow>, DbError>;
    fn get_block_pow(&self, block_id: &L1BlockId) -> Result<Option<U256>, DbError>;
    fn get_client_state(&self, block_id: &L1BlockId) -> Result<Option<L1ClientState>, DbError>;

    fn expect_block(&self, block_id: &L1BlockId) -> L1Block;
    fn expect_block_with_pow(&self, block_id: &L1BlockId) -> L1BlockWithPow;
    fn expect_block_pow(&self, block_id: &L1BlockId) -> U256;
    fn expect_client_state(&self, block_id: &L1BlockId) -> L1ClientState;

    fn save_client_state(&self, block_id: L1BlockId, state: L1ClientState) -> Result<(), DbError>;
    fn mark_block_valid(
        &self,
        block_id: &L1BlockId,
        height: u64,
        accumulated_pow: U256,
    ) -> Result<(), DbError>;
    fn remove_invalid_block(&self, block_id: &L1BlockId) -> Result<(), DbError>;
}

fn make_chain_context(storage: Arc<NodeStorage>) -> impl L1ChainContext {
    DbL1ChainContext { storage }
}

struct DbL1ChainContext {
    storage: Arc<NodeStorage>,
}

impl L1ChainContext for DbL1ChainContext {
    fn get_block(&self, block_id: &L1BlockId) -> Result<Option<L1Block>, DbError> {
        self.storage.l1().get_block_blocking(block_id)
    }

    fn get_block_with_pow(&self, block_id: &L1BlockId) -> Result<Option<L1BlockWithPow>, DbError> {
        match (self.get_block(block_id), self.get_block_pow(block_id)) {
            (Ok(Some(block)), Ok(Some(pow))) => Ok(Some(L1BlockWithPow::from_block(block, pow))),
            (Ok(None), _) => Ok(None),
            (Ok(Some(_)), Ok(None)) => {
                error!(%block_id, "csm: missing block pow");
                Ok(None)
            }
            (Err(err), _) => Err(err),
            (_, Err(err)) => Err(err),
        }
    }

    fn get_block_pow(&self, block_id: &L1BlockId) -> Result<Option<U256>, DbError> {
        self.storage
            .l1()
            .get_block_pow_blocking(block_id)
            .map(|maybe_pow| maybe_pow.map(|pow| U256::from_be_bytes(pow)))
    }

    fn get_client_state(&self, block_id: &L1BlockId) -> Result<Option<L1ClientState>, DbError> {
        self.storage.client_state().get_l1_state_blocking(block_id)
    }

    fn expect_block(&self, block_id: &L1BlockId) -> L1Block {
        self.get_block(block_id)
            .expect("csm: db error")
            .expect("csm: missing block")
    }

    fn expect_block_with_pow(&self, block_id: &L1BlockId) -> L1BlockWithPow {
        self.get_block_with_pow(block_id)
            .expect("csm: db error")
            .expect("csm: missing block")
    }

    fn expect_block_pow(&self, block_id: &L1BlockId) -> U256 {
        self.get_block_pow(block_id)
            .expect("csm: db error")
            .expect("csm: missing block pow")
    }

    fn expect_client_state(&self, block_id: &L1BlockId) -> L1ClientState {
        self.get_client_state(block_id)
            .expect("csm: db error")
            .expect("csm: missing client state")
    }

    fn save_client_state(&self, block_id: L1BlockId, state: L1ClientState) -> Result<(), DbError> {
        self.storage
            .client_state()
            .put_l1_state_blocking(block_id, state)
    }

    fn mark_block_valid(
        &self,
        block_id: &L1BlockId,
        height: u64,
        accumulated_pow: U256,
    ) -> Result<(), DbError> {
        self.storage
            .l1()
            .mark_block_valid_blocking(block_id, height, accumulated_pow.to_be_bytes())
    }

    fn remove_invalid_block(&self, block_id: &L1BlockId) -> Result<(), DbError> {
        self.storage.l1().remove_invalid_block_blocking(block_id)
    }
}

enum BlockStatus {
    Valid(L1ClientState),
    Invalid, // TODO: reason ?
}

fn client_stf(
    prev_state: &L1ClientState,
    block: &L1Block,
    _ctx: &impl L1ChainContext,
) -> anyhow::Result<BlockStatus> {
    let anchor_state = asm_stf(prev_state.anchor_state(), block.inner())?;

    Ok(BlockStatus::Valid(L1ClientState::new(
        block.block_id(),
        anchor_state,
    )))
}

fn asm_stf(prev_state: &AnchorState, _block: &Block) -> anyhow::Result<AnchorState> {
    // TODO: placeholder
    Ok(prev_state.clone())
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct U256(pub u128, pub u128); // (high, low)

impl U256 {
    /// Construct from a big-endian [u8; 32]
    pub fn from_be_bytes(bytes: [u8; 32]) -> Self {
        let high = u128::from_be_bytes(bytes[0..16].try_into().unwrap());
        let low = u128::from_be_bytes(bytes[16..32].try_into().unwrap());
        U256(high, low)
    }

    /// Convert back to [u8; 32] big-endian
    pub fn to_be_bytes(&self) -> [u8; 32] {
        let mut out = [0u8; 32];
        out[0..16].copy_from_slice(&self.0.to_be_bytes());
        out[16..32].copy_from_slice(&self.1.to_be_bytes());
        out
    }

    /// Saturating addition
    pub fn saturating_add(self, other: U256) -> U256 {
        let (low, carry) = self.1.overflowing_add(other.1);
        let (high, overflow) = self.0.overflowing_add(other.0 + (carry as u128));
        if overflow {
            U256(u128::MAX, u128::MAX) // saturate to max
        } else {
            U256(high, low)
        }
    }

    pub fn zero() -> Self {
        U256(0, 0)
    }
}

// Implement Ord and PartialOrd for comparison
impl Ord for U256 {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.cmp(&other.0).then(self.1.cmp(&other.1))
    }
}

impl PartialOrd for U256 {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
