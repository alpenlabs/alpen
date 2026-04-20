//! Reth ExEx that populates [`ExecBlockStorage`] with an [`ExecBlockRecord`]
//! for every canonical block the fullnode imports.
//!
//! The fullnode receives EE blocks via Reth p2p, but until STR-3076 nothing
//! materialised the per-block [`ExecBlockRecord`] the sequencer writes during
//! block production. Without those records `best_finalized_block` never
//! advanced and every downstream consumer of `ExecBlockStorage` (notably the
//! `alpen_getBlockStatus` RPC) degraded to a workaround.
//!
//! This ExEx closes the gap by reacting to Reth's canonical-chain
//! notifications, reusing the shared assembly core
//! ([`alpen_ee_block_assembly::assemble_next_exec_block_record`]) with a
//! replay-mode payload engine ([`crate::RethReplayPayloadEngine`]), and
//! persisting the result. The sequencer keeps writing records from its own
//! block-builder path; the ExEx is installed only on non-sequencer nodes (see
//! `bin/alpen-client/src/main.rs`).
//!
//! ## Scope in this revision
//!
//! Committed-chain path only — reorg handling via `reverted_chain` is a
//! follow-up. Inbox messages are fetched from the OL tracker's current best
//! finalized block; if the OL tracker has advanced past the slot range the
//! sequencer originally consumed, the replayed record's `ExecInputs` may
//! diverge from the sequencer's for that block. The exec block hash
//! (identifier) is always correct because it mirrors Reth's imported block.

use std::{num::NonZero, sync::Arc};

use alloy_rpc_types::BlockNumHash;
use alpen_ee_block_assembly::{
    assemble_next_exec_block_record, AssembleExecBlockInputs, AssembledExecBlock,
};
use alpen_ee_common::{
    get_inbox_messages_checked, ConsensusHeads, ExecBlockStorage, OLFinalizedStatus, OLInboxClient,
};
use alpen_ee_exec_chain::ExecChainHandle;
use futures_util::TryStreamExt;
use reth_exex::{ExExContext, ExExEvent};
use reth_node_api::{FullNodeComponents, NodeTypes};
use reth_primitives::EthPrimitives;
use reth_provider::Chain;
use strata_acct_types::{AccountId, Hash, MessageEntry};
use tokio::sync::watch;
use tracing::{debug, error, warn};

use crate::replay_payload_engine::RethReplayPayloadEngine;

/// Parameters the record generator pulls from outside — the two block-
/// assembly constants that live in `BlockBuilderConfig` on the sequencer
/// crate. They're passed in explicitly so this crate doesn't need a sequencer
/// dep.
#[derive(Debug, Clone)]
pub struct EeRecordGeneratorConfig {
    /// Max number of deposits per block. Must match the sequencer's value —
    /// determines how many pending inputs the replayed block drains.
    pub max_deposits_per_block: NonZero<u8>,
    /// Bridge gateway account id on OL. Must match the sequencer's value.
    pub bridge_gateway_account_id: AccountId,
}

/// ExEx that replays canonical EE blocks into [`ExecBlockStorage`].
///
/// Generic over the node components (standard Reth ExEx shape), the storage
/// implementation (`S`), and the OL inbox-message client (`O`). On the
/// fullnode the last two are `EeNodeStorage` and `OLClientKind`.
#[expect(
    missing_debug_implementations,
    reason = "ExExContext and provider types do not implement Debug"
)]
pub struct EeRecordGenerator<Node, S, O>
where
    Node: FullNodeComponents<Types: NodeTypes<Primitives = EthPrimitives>>,
    Node::Provider: Clone,
    S: ExecBlockStorage + Send + Sync + 'static,
    O: OLInboxClient + Send + Sync + 'static,
{
    ctx: ExExContext<Node>,
    storage: Arc<S>,
    ol_client: Arc<O>,
    exec_chain_handle: ExecChainHandle,
    ol_status_rx: watch::Receiver<OLFinalizedStatus>,
    // Held for step 6 (reorg handling via `reverted_chain`) — the reorg path
    // will consult consensus heads to decide what to roll back.
    #[expect(
        dead_code,
        reason = "reserved for reorg handling in step 6 of STR-3076"
    )]
    consensus_rx: watch::Receiver<ConsensusHeads>,
    config: EeRecordGeneratorConfig,
    payload_engine: RethReplayPayloadEngine<Node::Provider>,
}

impl<Node, S, O> EeRecordGenerator<Node, S, O>
where
    Node: FullNodeComponents<Types: NodeTypes<Primitives = EthPrimitives>>,
    Node::Provider: Clone
        + reth_provider::BlockReader<
            Block = reth_ethereum_primitives::Block,
            Receipt = reth_primitives::Receipt,
        > + reth_provider::ReceiptProvider<Receipt = reth_primitives::Receipt>
        + reth_provider::BlockNumReader
        + reth_provider::BlockHashReader
        + Send
        + Sync
        + 'static,
    S: ExecBlockStorage + Send + Sync + 'static,
    O: OLInboxClient + Send + Sync + 'static,
{
    pub fn new(
        ctx: ExExContext<Node>,
        storage: Arc<S>,
        ol_client: Arc<O>,
        exec_chain_handle: ExecChainHandle,
        ol_status_rx: watch::Receiver<OLFinalizedStatus>,
        consensus_rx: watch::Receiver<ConsensusHeads>,
        config: EeRecordGeneratorConfig,
    ) -> Self {
        let payload_engine = RethReplayPayloadEngine::new(ctx.provider().clone());
        Self {
            ctx,
            storage,
            ol_client,
            exec_chain_handle,
            ol_status_rx,
            consensus_rx,
            config,
            payload_engine,
        }
    }

    /// Process one committed chain: for each block in order, assemble and
    /// persist the matching [`ExecBlockRecord`]. Returns the highest block
    /// successfully committed, which the caller uses to emit a
    /// `FinishedHeight` event.
    async fn commit(&self, chain: &Chain) -> eyre::Result<Option<BlockNumHash>> {
        let mut highest = None;
        let blocks = chain.blocks();

        for block_number in chain.range() {
            let Some(block) = blocks.get(&block_number) else {
                continue;
            };
            let block_hash = block.hash();
            let hash_ref: Hash = block_hash.0.into();

            if self.storage.get_exec_block(hash_ref).await?.is_some() {
                // Already have a record (e.g. genesis; or a previous ExEx run
                // that committed up to here). Skip — `save_exec_block` is
                // idempotent but there's no reason to redo the work.
                highest = Some(BlockNumHash::new(block_number, block_hash));
                continue;
            }

            let parent_hash: Hash = block.parent_hash.0.into();
            let Some(parent_record) = self.storage.get_exec_block(parent_hash).await? else {
                // Gap in stored chain — we must process blocks in parent→child
                // order. Stop here; the next notification will retry.
                warn!(
                    %block_number,
                    parent = %block.parent_hash,
                    "ee_record_exex: parent record missing; skipping rest of chain"
                );
                break;
            };

            let best_ol_block = self.ol_status_rx.borrow().ol_block;
            let parent_ol = *parent_record.ol_block();

            // Only fetch inbox messages if OL has advanced past the parent's
            // pinned OL block. Matches the sequencer's `should_fetch_inbox_messages`
            // gate in `block_builder/task.rs`. See module docs for the caveat
            // on potential divergence from sequencer-consumed slot ranges.
            let (inbox_messages, next_inbox_msg_idx) = if parent_ol.blkid() != best_ol_block.blkid()
                && best_ol_block.slot() > parent_ol.slot()
            {
                match get_inbox_messages_checked(
                    self.ol_client.as_ref(),
                    parent_ol.slot(),
                    best_ol_block.slot(),
                )
                .await
                {
                    Ok(blocks) => {
                        let mut iter = blocks.into_iter();
                        // First block is the anchor at parent_ol.slot — already
                        // processed by an earlier block, skip it.
                        let _anchor = iter.next();
                        let mut messages: Vec<MessageEntry> = Vec::new();
                        let mut last_next_idx = parent_record.next_inbox_msg_idx();
                        for block_data in iter {
                            messages.extend(block_data.inbox_messages);
                            last_next_idx = block_data.next_inbox_msg_idx;
                        }
                        (messages, last_next_idx)
                    }
                    Err(err) => {
                        error!(
                            ?err,
                            %block_number,
                            "ee_record_exex: failed to fetch inbox messages; retrying later"
                        );
                        break;
                    }
                }
            } else {
                (vec![], parent_record.next_inbox_msg_idx())
            };

            let AssembledExecBlock {
                record,
                payload,
                blockhash,
            } = assemble_next_exec_block_record(
                AssembleExecBlockInputs {
                    parent_record: &parent_record,
                    inbox_messages,
                    next_inbox_msg_idx,
                    best_ol_block,
                    timestamp_ms: block.timestamp * 1_000,
                    max_deposits_per_block: self.config.max_deposits_per_block,
                    bridge_gateway_account_id: self.config.bridge_gateway_account_id,
                },
                &self.payload_engine,
            )
            .await?;

            self.storage.save_exec_block(record, payload).await?;
            if let Err(err) = self.exec_chain_handle.new_block(blockhash).await {
                // The exec-chain task is shared; if it's gone we can't recover,
                // but the record is saved so finality can still advance via
                // the OL-tracker path once the task restarts.
                error!(?err, ?blockhash, "ee_record_exex: exec chain notify failed");
            }

            debug!(%block_number, ?blockhash, "ee_record_exex: persisted record");
            highest = Some(BlockNumHash::new(block_number, block_hash));
        }

        Ok(highest)
    }

    pub async fn start(mut self) -> eyre::Result<()> {
        debug!("ee_record_exex: starting");
        while let Some(notification) = self.ctx.notifications.try_next().await? {
            if let Some(committed_chain) = notification.committed_chain() {
                match self.commit(&committed_chain).await {
                    Ok(Some(finished_height)) => {
                        self.ctx
                            .events
                            .send(ExExEvent::FinishedHeight(finished_height))?;
                    }
                    Ok(None) => {}
                    Err(err) => {
                        error!(
                            ?err,
                            "ee_record_exex: commit failed; awaiting next notification"
                        );
                    }
                }
            }
        }
        Ok(())
    }
}
