use std::collections::{HashMap, VecDeque};

use alpen_ee_common::{get_inbox_messages_checked, ExecBlockStorage, SequencerOLClient};
use eyre::eyre;
use strata_identifiers::{OLBlockCommitment, OLBlockId};
use strata_snark_acct_types::MessageEntry;
use tracing::{error, warn};

#[derive(Debug)]
pub struct OLChainTrackerState {
    /// Lowest block being tracked.
    /// The messages upto this block have already been processed.
    base: OLBlockCommitment,
    /// blocks whose messages have not been processed.
    blocks: VecDeque<OLBlockCommitment>,
    /// messages in the blocks.
    data: HashMap<OLBlockId, Vec<MessageEntry>>,
}

impl OLChainTrackerState {
    fn new_empty(base: OLBlockCommitment) -> Self {
        Self {
            base,
            blocks: VecDeque::new(),
            data: HashMap::new(),
        }
    }

    pub(crate) fn best_block(&self) -> OLBlockCommitment {
        *self.blocks.back().unwrap_or(&self.base)
    }

    pub(crate) fn append_block(
        &mut self,
        block: OLBlockCommitment,
        inbox_messages: Vec<MessageEntry>,
    ) -> eyre::Result<()> {
        if block.slot() != self.best_block().slot() + 1 {
            return Err(eyre!("invalid block; block must extend existing chain"));
        }

        self.blocks.push_back(block);
        self.data.insert(*block.blkid(), inbox_messages);

        Ok(())
    }

    pub(crate) fn prune_blocks(&mut self, next_base: OLBlockCommitment) -> eyre::Result<()> {
        if next_base == self.base {
            // noop
            return Ok(());
        }

        let Ok(prune_idx) = self.blocks.binary_search(&next_base) else {
            // not a tracked block
            return Err(eyre!("unknown block: {next_base:?}"));
        };

        self.base = next_base;
        for _ in 0..=prune_idx {
            let block = self.blocks.pop_front().expect("should exist");
            self.data.remove(block.blkid());
        }

        Ok(())
    }

    pub(crate) fn get_inbox_messages(
        &self,
        mut from_slot: u64,
        mut to_slot: u64,
    ) -> eyre::Result<Vec<MessageEntry>> {
        if from_slot > to_slot {
            return Err(eyre!(
                "invalid queryl from > to; from = {from_slot}, to = {to_slot}"
            ));
        }

        let (min_slot, max_slot) = match (self.blocks.front(), self.blocks.back()) {
            (Some(min_block), Some(max_block)) => (min_block.slot(), max_block.slot()),
            _ => {
                warn!("requested inbox messages from empty tracker");
                return Ok(vec![]);
            }
        };
        if from_slot < min_slot {
            warn!(
                min = min_slot,
                requested = from_slot,
                "requested inbox messages below min slot"
            );
            from_slot = min_slot;
        }
        if to_slot < max_slot {
            warn!(
                max = max_slot,
                requested = to_slot,
                "requested inbox messages above max slot"
            );
            to_slot = max_slot;
        }

        let valid_blocks = self
            .blocks
            .iter()
            .filter(|b| from_slot <= b.slot() && b.slot() <= to_slot);

        let mut res = Vec::new();
        for block in valid_blocks {
            let inbox_messages = self.data.get(block.blkid()).ok_or(eyre!(
                "missing inbox data for block ({}, {})",
                block.slot(),
                block.blkid()
            ))?;

            res.reserve(inbox_messages.len());
            for message in inbox_messages {
                res.push(message.clone());
            }
        }

        Ok(res)
    }
}

pub async fn init_ol_chain_tracker_state<TStorage: ExecBlockStorage, TClient: SequencerOLClient>(
    storage: &TStorage,
    ol_client: &TClient,
) -> eyre::Result<OLChainTrackerState> {
    // last finalized block known to EE sequencer locally
    let finalized_exec_block = storage
        .best_finalized_block()
        .await?
        .ok_or(eyre!("finalized block missing"))?;
    let local_finalized_ol_block = *finalized_exec_block.ol_block();

    let mut state = OLChainTrackerState::new_empty(local_finalized_ol_block);

    // chain status according to OL
    // TODO: retry
    let ol_chain_status = ol_client.chain_status().await?;
    let remote_finalized_ol_block = ol_chain_status.finalized().to_block_commitment();

    if remote_finalized_ol_block == local_finalized_ol_block {
        // no new finalized blocks available to be processed.
        return Ok(state);
    }

    if remote_finalized_ol_block.slot() < local_finalized_ol_block.slot() {
        // Block height that is considered finalized locally is not considered finalized on OL.
        //
        // Either a deep reorg has occurred on OL,
        // or a significant mismatch between OL and EE.
        // In either case, exit to avoid corrupting local data and await manual resolution.
        error!(
            local = ?local_finalized_ol_block,
            remote = ?remote_finalized_ol_block,
            "local finalized OL block ahead of OL"
        );
        return Err(eyre!(
            "local finalized state is ahead of connected OL's finalized slot"
        ));
    }

    // TODO: retry
    // TODO: chunk calls by slot range
    let blocks = get_inbox_messages_checked(
        ol_client,
        local_finalized_ol_block.slot(),
        remote_finalized_ol_block.slot(),
    )
    .await?;

    let (block_at_finalized_height, blocks) = {
        let mut iter = blocks.into_iter();
        let first = iter.next().expect("checked");

        (first, iter)
    };

    if block_at_finalized_height.commitment != local_finalized_ol_block {
        // The block we know to be finalized locally is not present in the OL chain.
        // OL chain has seen a deep reorg.
        // Avoid corrupting local data and exit to await manual resolution.
        error!(
            local = ?local_finalized_ol_block,
            remote = ?block_at_finalized_height.commitment,
            "local finalized OL block not present in OL"
        );

        return Err(eyre!(
            "local finalized state not present in OL chain. Deep reorg detected."
        ));
    }

    // Everything looks ok now. Build local state.
    for block in blocks {
        state.append_block(block.commitment, block.inbox_messages)?;
    }

    Ok(state)
}
