use std::sync::Arc;

use alloy_rpc_types::BlockNumHash;
use alpen_reth_db::StateDiffStore;
use futures_util::TryStreamExt;
use reth_exex::{ExExContext, ExExEvent};
use reth_node_api::{FullNodeComponents, NodeTypes};
use reth_primitives::EthPrimitives;
use reth_provider::{BlockReaderIdExt, Chain};
use tracing::{debug, error};

#[expect(missing_debug_implementations)]
pub struct StateDiffGenerator<
    Node: FullNodeComponents<Types: NodeTypes<Primitives = EthPrimitives>>,
    S: StateDiffStore + Clone,
> {
    ctx: ExExContext<Node>,
    db: Arc<S>,
}

impl<
        Node: FullNodeComponents<Types: NodeTypes<Primitives = EthPrimitives>>,
        S: StateDiffStore + Clone,
    > StateDiffGenerator<Node, S>
{
    pub fn new(ctx: ExExContext<Node>, db: Arc<S>) -> Self {
        Self { ctx, db }
    }

    fn commit(&mut self, chain: &Chain) -> eyre::Result<Option<BlockNumHash>> {
        let mut finished_height = None;
        let blocks = chain.blocks();
        let bundles = chain.range().filter_map(|block_number| {
            blocks
                .get(&block_number)
                .map(|block| block.hash())
                .zip(chain.execution_outcome_at_block(block_number))
        });

        for (block_hash, outcome) in bundles {
            #[cfg(debug_assertions)]
            assert!(outcome.len() == 1, "should only contain single block");
            let state_diff = outcome.bundle.into();

            // fetch current block
            let current_block = self
                .ctx
                .provider()
                .header_by_id(block_hash.into())?
                .ok_or_else(|| eyre::eyre!("block not found for hash {:?}", block_hash))?;
            let current_block_idx: u64 = current_block.number;

            // TODO: maybe put db writes in another thread
            if let Err(err) = self
                .db
                .put_state_diff(block_hash, current_block_idx, &state_diff)
            {
                error!(?err, ?block_hash);
                break;
            }

            finished_height = Some(BlockNumHash::new(current_block_idx, block_hash))
        }

        Ok(finished_height)
    }

    pub async fn start(mut self) -> eyre::Result<()> {
        debug!("start state diff generator");
        while let Some(notification) = self.ctx.notifications.try_next().await? {
            if let Some(committed_chain) = notification.committed_chain() {
                let finished_height = self.commit(&committed_chain)?;
                if let Some(finished_height) = finished_height {
                    self.ctx
                        .events
                        .send(ExExEvent::FinishedHeight(finished_height))?;
                }
            }
        }

        Ok(())
    }
}
