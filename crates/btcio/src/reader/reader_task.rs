use std::{ops::Range, sync::Arc, time::Duration};

use bitcoin::{Block, BlockHash};
use bitcoind_async_client::traits::Reader;
use futures::{
    stream::{unfold, FuturesUnordered},
    Stream, StreamExt,
};
use strata_config::btcio::ReaderConfig;
use strata_primitives::{l1::L1Block, params::Params};
use strata_state::l1::L1BlockId;
use strata_storage::{L1BlockManager, NodeStorage};
use tokio::{pin, select, sync::mpsc, time::sleep, try_join};
use tracing::warn;

pub enum ReaderCommand {
    FetchBlockById(L1BlockId),
    FetchBlockRange(Range<usize>),
}

pub async fn reader_task(
    client: Arc<impl Reader>,
    storage: Arc<NodeStorage>,
    params: Arc<Params>,
    config: Arc<ReaderConfig>,
    block_tx: mpsc::Sender<L1BlockId>,
    mut command_rx: mpsc::Receiver<ReaderCommand>,
) -> anyhow::Result<()> {
    // TODO: replace stream with zmq block listener
    let start_height = storage
        .l1()
        .get_best_valid_block_height()
        .await?
        .map(|height| height + 1)
        .unwrap_or(params.rollup.genesis_l1_height);

    // TODO: clean db entries of this case during startup checks
    assert!(
        start_height >= params.rollup.genesis_l1_height,
        "btcio: Invalid block reader start height"
    );

    let block_stream = get_block_stream(
        start_height,
        config.client_poll_dur_ms as u64,
        client.clone(),
    );
    pin!(block_stream);

    let (process_tx, mut process_rx) = mpsc::channel::<L1Block>(8);
    tokio::spawn(async move {
        while let Some(block) = process_rx.recv().await {
            let block_id = block.block_id();
            if let Err(err) = handle_block(block, storage.l1().as_ref(), &block_tx).await {
                warn!(%err, %block_id, "btcio: failed to process block");
            }
        }
    });

    loop {
        select! {
            block = block_stream.next() => {
                let (height, block) = block.expect("block must exist");
                let l1block = L1Block::new(height, block);

                let _ = process_tx.send(l1block).await;
                continue;
            }

            Some(command) = command_rx.recv() => {
                handle_command(command, client.as_ref(), &process_tx).await;
                continue;
            }
        }
    }
}

fn get_block_stream(
    start_height: u64,
    poll_interval_ms: u64,
    client: Arc<impl Reader>,
) -> impl Stream<Item = (u64, Block)> {
    unfold((start_height, client), move |(height, client)| async move {
        loop {
            match client.get_block_at(height).await {
                Ok(block) => return Some(((height, block), (height + 1, client))),
                Err(err) => {
                    warn!(%err, %height, "btcio: failed to fetch block");
                    // TODO: distinguish recoverable and non-recoverable error?
                    sleep(Duration::from_millis(poll_interval_ms)).await;
                }
            };
        }
    })
}

async fn handle_command(
    command: ReaderCommand,
    client: &impl Reader,
    process_tx: &mpsc::Sender<L1Block>,
) {
    match command {
        ReaderCommand::FetchBlockById(block_id) => {
            let blockhash = BlockHash::from(block_id);
            let block = match try_join!(
                client.get_block_height(&blockhash),
                client.get_block(&blockhash),
            ) {
                Ok((height, block)) => L1Block::new(height, block),
                Err(err) => {
                    warn!(%err, %block_id, "btcio: failed to fetch block");
                    return;
                }
            };
            let _ = process_tx.send(block).await;
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
                    Ok(block) => L1Block::new(height as u64, block),
                    Err(err) => {
                        warn!(%err, %height, "btcio: failed to fetch block");
                        return;
                    }
                };
                let _ = process_tx.send(block).await;
            }
        }
    }
}

async fn handle_block(
    block: L1Block,
    l1: &L1BlockManager,
    tx: &mpsc::Sender<L1BlockId>,
) -> anyhow::Result<()> {
    let blockid = block.block_id();
    l1.put_block_pending_validation(block).await?;
    let _ = tx.send(blockid).await;
    Ok(())
}
