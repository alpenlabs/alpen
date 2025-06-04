use std::{collections::VecDeque, sync::Arc};

use bitcoin::Block;
use strata_btcio::reader::reader_task::ReaderCommand;
use strata_db::DbError;
use strata_primitives::l1::L1Block;
use strata_state::{
    client_state::{AnchorState, L1ClientState},
    l1::L1BlockId,
};
use strata_storage::NodeStorage;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use super::{
    chain_tracker::{init_chain_tracker, AttachBlockResult},
    common::{L1Header, U256},
    orphan_tracker::OrphanTracker,
};

const MAX_RETRIES: u8 = 10;
const BATCH_FETCH_THRESHOLD: u64 = 5;

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
    let mut orphan_tracker = OrphanTracker::default();
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
                    // could indicate a reorg larger than our configured safety depth
                    warn!(block_id = %block.block_id(), height = %block.height(), safe_height = %chain_tracker.safe_height(), "csm: block below safe height");
                    continue;
                }
                AttachBlockResult::Duplicate => {
                    warn!(block_id = %block.block_id(), "csm: duplicate block");
                    continue;
                }
                AttachBlockResult::Orphan => {
                    info!(block_id = %block.block_id(), parent_id = %block.parent_id(), "csm: orphan block");

                    // try to fetch parent block. this will eventually reach a block in the known
                    // chain to attach the orphans to, or end at `BelowSafeHeight`
                    let _ =
                        command_tx.blocking_send(ReaderCommand::FetchBlockById(block.parent_id()));

                    let best_block_height = chain_tracker.best().height();
                    let block_height = block.height();

                    if block_height.saturating_sub(best_block_height) > BATCH_FETCH_THRESHOLD {
                        // we are very behind. queue all missing blocks to be fetched by height.
                        let _ = command_tx.blocking_send(ReaderCommand::FetchBlockRange(
                            (best_block_height + 1)..block_height,
                        ));
                    }

                    orphan_tracker.insert(&block);
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
                    let is_new_best = chain_tracker
                        .attach_block_unchecked(L1Header::from_block(&block, accumulated_pow));

                    if is_new_best {
                        // TODO: emit event for new best chainstate
                    }

                    // check if any orphan blocks can be attached to this block
                    if let Some(children) = orphan_tracker.children(&block_id).cloned() {
                        // add them to be processed next, in same relative order as the blocks were
                        // originally seen.
                        for child in children.iter().rev() {
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
    fn get_block_pow(&self, block_id: &L1BlockId) -> Result<Option<U256>, DbError>;
    fn get_client_state(&self, block_id: &L1BlockId) -> Result<Option<L1ClientState>, DbError>;

    fn expect_block(&self, block_id: &L1BlockId) -> L1Block;
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

    fn get_block_pow(&self, block_id: &L1BlockId) -> Result<Option<U256>, DbError> {
        self.storage
            .l1()
            .get_block_pow_blocking(block_id)
            .map(|maybe_pow| maybe_pow.map(U256::from_be_bytes))
    }

    fn get_client_state(&self, block_id: &L1BlockId) -> Result<Option<L1ClientState>, DbError> {
        self.storage.client_state().get_l1_state_blocking(block_id)
    }

    fn expect_block(&self, block_id: &L1BlockId) -> L1Block {
        self.get_block(block_id)
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
    Valid(L1ClientState /* , additional fields to save */),
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
