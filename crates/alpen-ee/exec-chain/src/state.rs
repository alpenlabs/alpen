use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};

use alpen_ee_common::{ExecBlockRecord, ExecBlockStorage};
use eyre::eyre;
use strata_acct_types::Hash;
use tracing::warn;

use crate::{
    orphan_tracker::OrphanTracker,
    unfinalized_tracker::{AttachBlockRes, UnfinalizedTracker},
};

#[derive(Debug)]
pub struct ExecChainState {
    /// unfinalized chains
    unfinalized: UnfinalizedTracker,
    /// orphan blocks
    orphans: OrphanTracker,
    /// cache block data
    blocks: HashMap<Hash, ExecBlockRecord>,
}

impl ExecChainState {
    /// Create new chain tracker with last finalized block
    pub(crate) fn new_empty(finalized_block: ExecBlockRecord) -> Self {
        Self {
            unfinalized: UnfinalizedTracker::new_empty((&finalized_block).into()),
            orphans: OrphanTracker::new_empty(),
            blocks: HashMap::from([(finalized_block.blockhash(), finalized_block)]),
        }
    }

    pub(crate) fn tip_blockhash(&self) -> Hash {
        self.unfinalized.best().hash
    }

    pub(crate) fn finalized_blockhash(&self) -> Hash {
        self.unfinalized.finalized().hash
    }

    pub(crate) fn append_block(&mut self, block: ExecBlockRecord) -> eyre::Result<Hash> {
        match self.unfinalized.attach_block((&block).into()) {
            AttachBlockRes::Ok(new_tip) => Ok(self.check_orphan_blocks(new_tip)),
            AttachBlockRes::BelowFinalized(_) => Err(eyre!("block height below finalized")),
            AttachBlockRes::ExistingBlock => {
                warn!("block already present in tracker");
                Ok(self.tip_blockhash())
            }
            AttachBlockRes::OrphanBlock(block_entry) => {
                self.orphans.insert(block_entry);
                Ok(self.tip_blockhash())
            }
        }
    }

    fn check_orphan_blocks(&mut self, mut tip: Hash) -> Hash {
        let mut attachable_blocks: VecDeque<_> = self.orphans.take_children(&tip).into();
        while let Some(block) = attachable_blocks.pop_front() {
            match self.unfinalized.attach_block(block) {
                AttachBlockRes::Ok(best) => {
                    tip = best;
                    attachable_blocks.append(&mut self.orphans.take_children(&tip).into());
                }
                AttachBlockRes::ExistingBlock => {
                    // shouldnt happen but safe to ignore
                    warn!("unexpected existing block");
                }
                AttachBlockRes::OrphanBlock(_) => unreachable!(),
                AttachBlockRes::BelowFinalized(_) => unreachable!(),
            }
        }

        tip
    }

    pub(crate) fn get_best_block(&self) -> &ExecBlockRecord {
        self.blocks
            .get(&self.unfinalized.best().hash)
            .expect("should exist")
    }

    pub(crate) fn contains_unfinalized_block(&self, hash: &Hash) -> bool {
        self.unfinalized.contains_block(hash)
    }

    pub(crate) fn contains_orphan_block(&self, hash: &Hash) -> bool {
        self.orphans.has_block(hash)
    }

    pub(crate) fn prune_finalized(&mut self, finalized: Hash) {
        let report = self
            .unfinalized
            .prune_finalized(finalized)
            .expect("checked");
        let finalized_height = self
            .blocks
            .get(&finalized)
            .expect("should exist")
            .blocknum();
        for hash in report.finalize {
            self.blocks.remove(&hash);
        }
        for hash in report.remove {
            self.blocks.remove(&hash);
        }
        let removed_orphans = self.orphans.purge_by_height(finalized_height);
        for hash in removed_orphans {
            self.blocks.remove(&hash);
        }
    }
}

/// Init state using last finalized block and all unfinalized blocks from storage.
pub async fn init_exec_chain_state_from_storage<TStorage: ExecBlockStorage>(
    storage: Arc<TStorage>,
) -> eyre::Result<ExecChainState> {
    let last_finalized_block = storage
        .best_finalized_block()
        .await?
        .expect("cannot be empty");

    let mut state = ExecChainState::new_empty(last_finalized_block);

    for blockhash in storage.get_unfinalized_blocks().await? {
        let block = storage
            .get_exec_block(blockhash)
            .await?
            .ok_or(eyre!("missing expected block: {:?}", blockhash))?;

        state.append_block(block)?;
    }

    Ok(state)
}
