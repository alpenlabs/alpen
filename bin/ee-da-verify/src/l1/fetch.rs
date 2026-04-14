//! Streams Bitcoin blocks from bitcoind with retry.

use std::{fmt::Debug, future::Future, pin::Pin};

use bitcoin::{Block, BlockHash};
use bitcoind_async_client::{error::ClientError, traits::Reader, ClientResult};
use futures::stream::{self, BoxStream, StreamExt};
use strata_common::retry::{policies::ExponentialBackoff, retry_with_backoff_async};
use strata_identifiers::L1Height;
use thiserror::Error;

/// Retry count for each bitcoind RPC call in the fetch stage.
pub(crate) const FETCH_MAX_RETRIES: u16 = 5;

/// Initial retry delay (milliseconds) for fetch-stage RPC calls.
pub(crate) const FETCH_BACKOFF_BASE_DELAY_MS: u64 = 500;

/// Retry delay multiplier numerator.
pub(crate) const FETCH_BACKOFF_MULTIPLIER: u64 = 150;

/// Retry delay multiplier denominator.
pub(crate) const FETCH_BACKOFF_MULTIPLIER_BASE: u64 = 100;

/// bitcoind RPC error code for "Block height out of range".
const BITCOIND_BLOCK_HEIGHT_OUT_OF_RANGE: i32 = -8;

/// bitcoind RPC error code for "Loading block index" (node warming up).
const BITCOIND_WARMING_UP: i32 = -28;

/// Raw Bitcoin block data paired with its fetch metadata.
#[derive(Debug, Clone)]
#[expect(
    dead_code,
    reason = "Downstream stages consume the full block payload and metadata."
)]
pub(crate) struct L1BlockData {
    pub(crate) height: L1Height,
    pub(crate) hash: BlockHash,
    pub(crate) block: Block,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
#[error("--start-height ({start_height}) must be <= --end-height ({end_height})")]
pub(crate) struct InvalidBlockRange {
    pub(crate) start_height: L1Height,
    pub(crate) end_height: L1Height,
}

#[derive(Debug, Error)]
pub(crate) enum FetchError {
    #[error("height out of range at {height}: {source}")]
    HeightOutOfRange {
        height: L1Height,
        #[source]
        source: ClientError,
    },

    #[error("retries exhausted at height {height}: {source}")]
    RetriesExhausted {
        height: L1Height,
        #[source]
        source: ClientError,
    },

    #[error("client error at height {height}: {source}")]
    Client {
        height: L1Height,
        #[source]
        source: ClientError,
    },
}

#[derive(Debug)]
enum RpcFetchOutcome<T> {
    Value(T),
    HeightOutOfRange(ClientError),
    RetriesExhausted(ClientError),
    Terminal(ClientError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetryClass {
    Retryable,
    Terminal,
}

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub(crate) trait FetchReader: Send + Sync {
    fn get_block_hash(&self, height: L1Height) -> BoxFuture<'_, ClientResult<BlockHash>>;

    fn get_block<'a>(&'a self, hash: &'a BlockHash) -> BoxFuture<'a, ClientResult<Block>>;
}

impl<T> FetchReader for T
where
    T: Reader + Send + Sync,
{
    fn get_block_hash(&self, height: L1Height) -> BoxFuture<'_, ClientResult<BlockHash>> {
        Box::pin(Reader::get_block_hash(self, u64::from(height)))
    }

    fn get_block<'a>(&'a self, hash: &'a BlockHash) -> BoxFuture<'a, ClientResult<Block>> {
        Box::pin(Reader::get_block(self, hash))
    }
}

/// Returns an ordered stream of Bitcoin blocks for the inclusive height range.
pub(crate) fn fetch_range<'a, R>(
    reader: &'a R,
    start_height: L1Height,
    end_height: L1Height,
) -> BoxStream<'a, Result<L1BlockData, FetchError>>
where
    R: FetchReader + ?Sized,
{
    if start_height > end_height {
        return stream::empty().boxed();
    }

    stream::iter(start_height..=end_height)
        .then(move |height| fetch_block_at(reader, height))
        .boxed()
}

fn fetch_backoff() -> ExponentialBackoff {
    ExponentialBackoff::new(
        FETCH_BACKOFF_BASE_DELAY_MS,
        FETCH_BACKOFF_MULTIPLIER,
        FETCH_BACKOFF_MULTIPLIER_BASE,
    )
}

fn classify_retry(err: &ClientError) -> RetryClass {
    match err {
        ClientError::Connection(_) | ClientError::Timeout | ClientError::Request(_) => {
            RetryClass::Retryable
        }
        ClientError::Server(BITCOIND_WARMING_UP, _) => RetryClass::Retryable,
        _ => RetryClass::Terminal,
    }
}

fn is_height_out_of_range(error: &ClientError) -> bool {
    matches!(
        error,
        ClientError::Server(BITCOIND_BLOCK_HEIGHT_OUT_OF_RANGE, _)
    )
}

async fn fetch_block_at<R>(reader: &R, height: L1Height) -> Result<L1BlockData, FetchError>
where
    R: FetchReader + ?Sized,
{
    let hash = fetch_block_hash_with_retry(reader, height).await?;
    let block = fetch_block_with_retry(reader, height, hash).await?;

    Ok(L1BlockData {
        height,
        hash,
        block,
    })
}

async fn fetch_block_hash_with_retry<R>(
    reader: &R,
    height: L1Height,
) -> Result<BlockHash, FetchError>
where
    R: FetchReader + ?Sized,
{
    let outcome = retry_with_backoff_async(
        "ee_da_verify_fetch_block_hash",
        FETCH_MAX_RETRIES,
        &fetch_backoff(),
        || async {
            match reader.get_block_hash(height).await {
                Ok(hash) => Ok(RpcFetchOutcome::Value(hash)),
                Err(err) if is_height_out_of_range(&err) => {
                    Ok(RpcFetchOutcome::HeightOutOfRange(err))
                }
                Err(err) if matches!(err, ClientError::MaxRetriesExceeded(_)) => {
                    Ok(RpcFetchOutcome::RetriesExhausted(err))
                }
                Err(err) => match classify_retry(&err) {
                    RetryClass::Retryable => Err(err),
                    RetryClass::Terminal => Ok(RpcFetchOutcome::Terminal(err)),
                },
            }
        },
    )
    .await
    .map_err(|source| FetchError::RetriesExhausted { height, source })?;

    match outcome {
        RpcFetchOutcome::Value(hash) => Ok(hash),
        RpcFetchOutcome::HeightOutOfRange(source) => {
            Err(FetchError::HeightOutOfRange { height, source })
        }
        RpcFetchOutcome::RetriesExhausted(source) => {
            Err(FetchError::RetriesExhausted { height, source })
        }
        RpcFetchOutcome::Terminal(source) => Err(FetchError::Client { height, source }),
    }
}

async fn fetch_block_with_retry<R>(
    reader: &R,
    height: L1Height,
    hash: BlockHash,
) -> Result<Block, FetchError>
where
    R: FetchReader + ?Sized,
{
    let outcome = retry_with_backoff_async(
        "ee_da_verify_fetch_block",
        FETCH_MAX_RETRIES,
        &fetch_backoff(),
        || async {
            match reader.get_block(&hash).await {
                Ok(block) => Ok(RpcFetchOutcome::Value(block)),
                Err(err) if matches!(err, ClientError::MaxRetriesExceeded(_)) => {
                    Ok(RpcFetchOutcome::RetriesExhausted(err))
                }
                Err(err) => match classify_retry(&err) {
                    RetryClass::Retryable => Err(err),
                    RetryClass::Terminal => Ok(RpcFetchOutcome::Terminal(err)),
                },
            }
        },
    )
    .await
    .map_err(|source| FetchError::RetriesExhausted { height, source })?;

    match outcome {
        RpcFetchOutcome::Value(block) => Ok(block),
        RpcFetchOutcome::HeightOutOfRange(_) => {
            unreachable!("get_block uses a hash, not a height")
        }
        RpcFetchOutcome::RetriesExhausted(source) => {
            Err(FetchError::RetriesExhausted { height, source })
        }
        RpcFetchOutcome::Terminal(source) => Err(FetchError::Client { height, source }),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{HashMap, VecDeque},
        sync::Mutex,
    };

    use bitcoin::{
        block::{Header, Version},
        hashes::Hash,
        pow::CompactTarget,
        Block, BlockHash, TxMerkleNode,
    };
    use bitcoind_async_client::error::ClientError;
    use futures::TryStreamExt;

    use super::*;

    #[derive(Default, Debug)]
    struct MockReader {
        hash_responses: Mutex<HashMap<L1Height, VecDeque<ClientResult<BlockHash>>>>,
        block_responses: Mutex<HashMap<BlockHash, VecDeque<ClientResult<Block>>>>,
        hash_calls: Mutex<Vec<L1Height>>,
    }

    impl MockReader {
        fn with_hash_responses(
            mut self,
            height: L1Height,
            responses: Vec<ClientResult<BlockHash>>,
        ) -> Self {
            self.hash_responses
                .get_mut()
                .expect("poisoned")
                .insert(height, responses.into_iter().collect());
            self
        }

        fn with_block_responses(
            mut self,
            hash: BlockHash,
            responses: Vec<ClientResult<Block>>,
        ) -> Self {
            self.block_responses
                .get_mut()
                .expect("poisoned")
                .insert(hash, responses.into_iter().collect());
            self
        }

        fn hash_call_count(&self, height: L1Height) -> usize {
            self.hash_calls
                .lock()
                .expect("poisoned")
                .iter()
                .filter(|h| **h == height)
                .count()
        }
    }

    impl FetchReader for MockReader {
        fn get_block_hash(&self, height: L1Height) -> BoxFuture<'_, ClientResult<BlockHash>> {
            self.hash_calls.lock().expect("poisoned").push(height);

            let response = self
                .hash_responses
                .lock()
                .expect("poisoned")
                .get_mut(&height)
                .and_then(|queue| queue.pop_front())
                .expect("missing hash response for height");

            Box::pin(async move { response })
        }

        fn get_block<'a>(&'a self, hash: &'a BlockHash) -> BoxFuture<'a, ClientResult<Block>> {
            let response = self
                .block_responses
                .lock()
                .expect("poisoned")
                .get_mut(hash)
                .and_then(|queue| queue.pop_front())
                .expect("missing block response for hash");

            Box::pin(async move { response })
        }
    }

    fn test_hash(byte: u8) -> BlockHash {
        BlockHash::from_byte_array([byte; 32])
    }

    fn test_block(nonce: u32) -> Block {
        Block {
            header: Header {
                version: Version::from_consensus(1),
                prev_blockhash: BlockHash::all_zeros(),
                merkle_root: TxMerkleNode::all_zeros(),
                time: 0,
                bits: CompactTarget::from_consensus(0),
                nonce,
            },
            txdata: Vec::new(),
        }
    }

    #[tokio::test]
    async fn fetch_range_preserves_height_ordering() {
        let heights: [L1Height; 3] = [10, 11, 12];
        let expected = heights
            .iter()
            .map(|height| (*height, test_hash(*height as u8)))
            .collect::<Vec<_>>();

        let mut reader = MockReader::default();
        for (height, hash) in &expected {
            reader = reader
                .with_hash_responses(*height, vec![Ok(*hash)])
                .with_block_responses(*hash, vec![Ok(test_block(*height))]);
        }

        let blocks = fetch_range(&reader, 10, 12)
            .try_collect::<Vec<L1BlockData>>()
            .await
            .expect("fetch must succeed");

        assert_eq!(blocks.len(), expected.len());
        for (block, (height, hash)) in blocks.iter().zip(expected.iter()) {
            assert_eq!(block.height, *height);
            assert_eq!(block.hash, *hash);
        }
    }

    #[tokio::test]
    async fn fetch_range_retries_then_succeeds_on_transient_error() {
        let hash = test_hash(7);

        let reader = MockReader::default()
            .with_hash_responses(
                7,
                vec![
                    Err(ClientError::Connection("temporary".to_string())),
                    Ok(hash),
                ],
            )
            .with_block_responses(hash, vec![Ok(test_block(7))]);

        let blocks = fetch_range(&reader, 7, 7)
            .try_collect::<Vec<L1BlockData>>()
            .await
            .expect("fetch must succeed after retry");

        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].height, 7);
        assert_eq!(reader.hash_call_count(7), 2);
    }

    #[tokio::test]
    async fn fetch_range_does_not_retry_terminal_auth_error() {
        let reader = MockReader::default().with_hash_responses(
            1,
            vec![Err(ClientError::Status(401, "Unauthorized".to_string()))],
        );

        let error = fetch_range(&reader, 1, 1)
            .try_collect::<Vec<L1BlockData>>()
            .await
            .expect_err("auth error must fail without retries");

        assert!(matches!(
            error,
            FetchError::Client {
                source: ClientError::Status(401, _),
                ..
            }
        ));
        assert_eq!(reader.hash_call_count(1), 1);
    }

    #[tokio::test]
    async fn fetch_range_maps_missing_height_to_height_out_of_range() {
        let reader = MockReader::default().with_hash_responses(
            42,
            vec![Err(ClientError::Server(
                -8,
                "Block height out of range".to_string(),
            ))],
        );

        let error = fetch_range(&reader, 42, 42)
            .try_collect::<Vec<L1BlockData>>()
            .await
            .expect_err("missing height must fail");

        assert!(matches!(
            error,
            FetchError::HeightOutOfRange {
                source: ClientError::Server(-8, _),
                ..
            }
        ));
        assert_eq!(reader.hash_call_count(42), 1);
    }

    #[tokio::test]
    async fn fetch_range_maps_retry_exhaustion_distinctly() {
        let responses = (0..16)
            .map(|_| Err(ClientError::Connection("temporary".to_string())))
            .collect::<Vec<_>>();
        let reader = MockReader::default().with_hash_responses(9, responses);

        let error = fetch_range(&reader, 9, 9)
            .try_collect::<Vec<L1BlockData>>()
            .await
            .expect_err("persistent retryable errors must exhaust retries");

        assert!(matches!(
            error,
            FetchError::RetriesExhausted {
                source: ClientError::Connection(_),
                ..
            }
        ));
    }

    #[tokio::test]
    async fn fetch_range_maps_client_max_retries_exceeded_to_retries_exhausted() {
        let reader = MockReader::default()
            .with_hash_responses(100, vec![Err(ClientError::MaxRetriesExceeded(3))]);

        let error = fetch_range(&reader, 100, 100)
            .try_collect::<Vec<L1BlockData>>()
            .await
            .expect_err("client max-retries error must map to retries exhausted");

        assert!(matches!(
            error,
            FetchError::RetriesExhausted {
                source: ClientError::MaxRetriesExceeded(3),
                ..
            }
        ));
    }
}
