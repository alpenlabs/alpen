use std::{ops::Range, sync::Arc, time::Duration};

use bitcoin::Block;
use bitcoind_async_client::traits::Reader;
use futures::{
    stream::{unfold, FuturesUnordered},
    Stream, StreamExt,
};
use strata_primitives::params::Params;
use strata_state::l1::L1BlockId;
use strata_storage::{L1BlockManager, NodeStorage};
use tokio::{select, sync::mpsc};
use tracing::{debug, error, warn};

pub enum ReaderCommand {
    FetchBlockById(L1BlockId),
    FetchBlockRange(Range<usize>),
}

pub async fn reader_task(
    client: Arc<impl Reader>,
    storage: Arc<NodeStorage>,
    params: Arc<Params>,
    block_tx: mpsc::Sender<L1BlockId>,
    mut command_rx: mpsc::Receiver<ReaderCommand>,
) -> anyhow::Result<()> {
    let start_height = storage
        .l1()
        .get_canonical_chain_tip_async()
        .await?
        .map(|(height, _)| height + 1)
        .unwrap_or(params.rollup.genesis_l1_height);

    let block_stream = get_block_stream(start_height, client.clone());
    tokio::pin!(block_stream);

    loop {
        select! {
            block = block_stream.next() => {
                let block = block.expect("block must exist");
                if let Err(err) = handle_block(block, storage.l1(), &block_tx).await {
                    warn!("failed to process block: {:?}", err);
                }
                continue;
            }

            Some(command) = command_rx.recv() => {
                handle_command(command, client.as_ref(), &block_tx, storage.l1()).await;
                continue;
            }
        }
    }
}

// TODO: replace with zmq block listener
fn get_block_stream(start_height: u64, client: Arc<impl Reader>) -> impl Stream<Item = Block> {
    unfold((start_height, client), |(next_height, client)| async move {
        loop {
            match client.get_block_at(next_height).await {
                Ok(block) => return Some((block, (next_height + 1, client.clone()))),
                Err(err) => {
                    dbg!(&err);
                    // TODO: distinguish recoverable and non-recoverable error
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            };
        }
    })
}

async fn save_block(block: Block, l1: &L1BlockManager) -> anyhow::Result<L1BlockId> {
    unimplemented!()
}

async fn handle_block(
    block: Block,
    l1: &L1BlockManager,
    tx: &mpsc::Sender<L1BlockId>,
) -> anyhow::Result<()> {
    let blockid = save_block(block, l1).await?;
    let _ = tx.send(blockid).await;
    Ok(())
}

async fn handle_command(
    command: ReaderCommand,
    client: &impl Reader,
    block_tx: &mpsc::Sender<L1BlockId>,
    l1: &L1BlockManager,
) {
    match command {
        ReaderCommand::FetchBlockById(block_id) => {
            let block = match client.get_block(&block_id.into()).await {
                Ok(block) => block,
                Err(err) => {
                    warn!(%block_id, "failed to fetch block: {:?}", err);
                    return;
                }
            };
            if let Err(err) = handle_block(block, l1, block_tx).await {
                warn!(%block_id, "failed to process block: {:?}", err);
            }
        }
        ReaderCommand::FetchBlockRange(range) => {
            let mut work = FuturesUnordered::new();
            for height in range {
                work.push(async move {
                    let res = client.get_block_at(height as u64).await;
                    (height, res)
                })
            }

            while let Some((height, res)) = work.next().await {
                let block = match res {
                    Ok(block) => block,
                    Err(err) => {
                        warn!(%height, "failed to fetch block: {:?}", err);
                        return;
                    }
                };
                if let Err(err) = handle_block(block, l1, block_tx).await {
                    warn!(%height, "failed to process block: {:?}", err);
                }
            }
        }
    }
}
