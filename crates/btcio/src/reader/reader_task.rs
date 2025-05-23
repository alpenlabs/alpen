use std::{ops::Range, sync::Arc, time::Duration};

use bitcoin::{Block, BlockHash};
use bitcoind_async_client::{error::ClientError, traits::Reader};
use futures::{
    stream::{unfold, FuturesUnordered},
    Stream, StreamExt,
};
use strata_config::btcio::ReaderConfig;
use strata_db::DbError;
use strata_primitives::{l1::L1Block, params::Params};
use strata_state::l1::L1BlockId;
use strata_storage::{L1BlockManager, NodeStorage};
use thiserror::Error;
use tokio::{pin, select, sync::mpsc, time::sleep, try_join};
use tracing::{error, info, warn};

pub enum ReaderCommand {
    FetchBlockById(L1BlockId),
    FetchBlockRange(Range<u64>),
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
                match err {
                    HandleBlockError::BlockTxChannelClosed => {
                        // not recoverable
                        return;
                    }
                    HandleBlockError::Db(_err) => {
                        // TODO: retry ?
                    }
                }
            }
        }
        info!("btcio: block_processor_handle finished as process_rx channel was closed.");
    });

    loop {
        select! {
            block_result = block_stream.next() => {
                match block_result {
                    Some((height, block)) => {
                        let l1block = L1Block::new(height, block);
                        if process_tx.send(l1block).await.is_err() {
                            error!("btcio: process_tx channel closed. Exiting reader_task.");
                            break;
                        }
                    }
                    None => {
                        warn!("btcio: block_stream ended unexpectedly. Exiting reader_task.");
                        break;
                    }
                }
            }

            Some(command) = command_rx.recv() => {
                if let Err(err) = handle_command(command, client.as_ref(), &process_tx).await {
                    match err {
                        HandleCommandError::ProcessTxChannelClosed => {
                            // not recoverable
                            break;
                        }
                        HandleCommandError::Client(_err) => {
                            // TODO: retry ?
                        }
                    }
                }
            }

            else => {
                info!("btcio: command_rx channel closed. Exiting reader_task.");
                break;
            }
        }
    }

    Err(anyhow::anyhow!("btcio: reader_task ended unexpectedly"))
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
                    // TODO: distinguish unrecoverable errors?
                    sleep(Duration::from_millis(poll_interval_ms)).await;
                }
            };
        }
    })
}

#[derive(Debug, Error)]
enum HandleCommandError {
    #[error("process_tx channel closed")]
    ProcessTxChannelClosed,
    #[error(transparent)]
    Client(#[from] ClientError),
}

async fn handle_command(
    command: ReaderCommand,
    client: &impl Reader,
    process_tx: &mpsc::Sender<L1Block>,
) -> Result<(), HandleCommandError> {
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
                    return Err(HandleCommandError::Client(err));
                }
            };
            if process_tx.send(block).await.is_err() {
                error!("btcio: process_tx channel closed while sending single fetched block. Reader task should exit.");
                return Err(HandleCommandError::ProcessTxChannelClosed);
            }
        }
        ReaderCommand::FetchBlockRange(range) => {
            let mut work = FuturesUnordered::new();
            for height in range {
                work.push(async move {
                    let res = client.get_block_at(height).await;
                    (height, res)
                })
            }

            while let Some((height, res)) = work.next().await {
                let block_content = match res {
                    Ok(block_content) => block_content,
                    Err(err) => {
                        warn!(%err, %height, "btcio: failed to fetch block in range for this height, skipping.");
                        continue;
                    }
                };
                let l1block = L1Block::new(height, block_content);
                if process_tx.send(l1block).await.is_err() {
                    error!(%height, "btcio: process_tx channel closed while sending block from range fetch for height. Reader task should exit.");
                    return Err(HandleCommandError::ProcessTxChannelClosed);
                }
            }
        }
    }
    Ok(())
}

#[derive(Debug, Error)]
enum HandleBlockError {
    #[error("block_tx channel closed")]
    BlockTxChannelClosed,
    #[error(transparent)]
    Db(#[from] DbError),
}

async fn handle_block(
    block: L1Block,
    l1: &L1BlockManager,
    tx: &mpsc::Sender<L1BlockId>,
) -> Result<(), HandleBlockError> {
    let block_id = block.block_id();
    l1.put_block_pending_validation(block).await?;
    if tx.send(block_id).await.is_err() {
        error!(%block_id, "btcio: failed to send block_id; channel closed");
        Err(HandleBlockError::BlockTxChannelClosed)?
    }
    Ok(())
}
