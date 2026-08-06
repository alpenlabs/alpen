//! Streams Bitcoin blocks from bitcoind with retry.

use std::time::Duration;

use bitcoin::{Block, BlockHash};
use bitcoind_async_client::{error::ClientError, traits::Reader, ClientResult};
use futures::{
    future::BoxFuture,
    stream::{self, BoxStream, StreamExt},
};
use strata_common::retry::{policies::ExponentialBackoff, Backoff};
use strata_identifiers::L1Height;
use thiserror::Error;
use tokio::time::sleep;

/// Maximum inclusive Bitcoin block count accepted by one extraction.
///
/// This bounds scanner memory use. Callers covering larger windows should
/// split the scan into multiple ranges.
pub const MAX_EXTRACTION_BLOCK_RANGE: u64 = 2_000;

/// bitcoind RPC error code for "Block height out of range".
const BITCOIND_BLOCK_HEIGHT_OUT_OF_RANGE: i32 = -8;

/// bitcoind RPC error code for "Loading block index" (node warming up).
const BITCOIND_WARMING_UP: i32 = -28;

/// Raw Bitcoin block data paired with its fetch metadata.
#[derive(Debug, Clone)]
pub struct L1BlockData {
    height: L1Height,
    hash: BlockHash,
    block: Block,
}

impl L1BlockData {
    /// Creates fetched L1 block data.
    pub fn new(height: L1Height, hash: BlockHash, block: Block) -> Self {
        Self {
            height,
            hash,
            block,
        }
    }

    /// Returns the L1 height where the block was fetched.
    pub fn height(&self) -> L1Height {
        self.height
    }

    /// Returns the Bitcoin block hash for the fetched block.
    pub fn hash(&self) -> BlockHash {
        self.hash
    }

    /// Returns the fetched Bitcoin block.
    pub fn block(&self) -> &Block {
        &self.block
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum FetchRangeError {
    #[error("start height {start_height} must be <= end height {end_height}")]
    Inverted {
        start_height: L1Height,
        end_height: L1Height,
    },

    #[error(
        "extraction range too large: requested {}, max {}",
        u64::from(*end_height - *start_height) + 1,
        MAX_EXTRACTION_BLOCK_RANGE,
    )]
    TooLarge {
        start_height: L1Height,
        end_height: L1Height,
    },
}

/// Ordered stream of per-block fetch results.
pub type FetchStream<'a> = BoxStream<'a, Result<L1BlockData, FetchError>>;

/// Result returned when constructing a bounded fetch stream.
pub type FetchRangeResult<'a> = Result<FetchStream<'a>, FetchRangeError>;

/// Retry policy for bounded L1 block fetches.
#[derive(Clone, Copy, Debug)]
pub struct FetchRetryPolicy {
    max_retries: u16,
    backoff: ExponentialBackoff,
}

impl FetchRetryPolicy {
    /// Creates a fetch retry policy.
    pub fn new(max_retries: u16, backoff: ExponentialBackoff) -> Self {
        Self {
            max_retries,
            backoff,
        }
    }

    /// Returns the maximum number of retries per RPC call.
    pub fn max_retries(&self) -> u16 {
        self.max_retries
    }

    /// Returns the retry backoff policy.
    pub fn backoff(&self) -> &ExponentialBackoff {
        &self.backoff
    }
}

#[derive(Debug, Error)]
pub enum FetchError {
    #[error("L1 block height out of range (height {height}): {source}")]
    HeightOutOfRange {
        height: L1Height,
        #[source]
        source: ClientError,
    },

    #[error(
        "L1 block fetch retries exhausted (height {height}, max retries {max_retries}): {source}"
    )]
    RetriesExhausted {
        height: L1Height,
        max_retries: u16,
        #[source]
        source: ClientError,
    },

    #[error("L1 block fetch failed (height {height}): {source}")]
    Client {
        height: L1Height,
        #[source]
        source: ClientError,
    },
}

/// Narrow adapter seam over the bitcoind block reader methods used by fetch.
///
/// This trait exists to unit-test fetch retry behavior without depending on a
/// live bitcoind instance. It is not intended as a broad extension API.
pub trait FetchReader: Send + Sync {
    /// Fetches the block at `height`.
    fn get_block_at(&self, height: L1Height) -> BoxFuture<'_, ClientResult<Block>>;
}

impl<T> FetchReader for T
where
    T: Reader + Send + Sync,
{
    fn get_block_at(&self, height: L1Height) -> BoxFuture<'_, ClientResult<Block>> {
        Box::pin(Reader::get_block_at(self, u64::from(height)))
    }
}

/// Returns an ordered stream of Bitcoin blocks using the supplied retry policy.
pub fn fetch_range<'a, R>(
    reader: &'a R,
    start_height: L1Height,
    end_height: L1Height,
    policy: FetchRetryPolicy,
) -> FetchRangeResult<'a>
where
    R: FetchReader + ?Sized,
{
    if start_height > end_height {
        return Err(FetchRangeError::Inverted {
            start_height,
            end_height,
        });
    }

    let requested_block_count = u64::from(end_height - start_height) + 1;
    if requested_block_count > MAX_EXTRACTION_BLOCK_RANGE {
        return Err(FetchRangeError::TooLarge {
            start_height,
            end_height,
        });
    }

    Ok(stream::iter(start_height..=end_height)
        .then(move |height| fetch_block_at(reader, height, policy))
        .boxed())
}

fn is_retryable_fetch_error(err: &ClientError) -> bool {
    matches!(
        err,
        ClientError::Connection(_)
            | ClientError::Timeout
            | ClientError::Request(_)
            | ClientError::Server(BITCOIND_WARMING_UP, _)
    )
}

fn is_height_out_of_range(error: &ClientError) -> bool {
    matches!(
        error,
        ClientError::Server(BITCOIND_BLOCK_HEIGHT_OUT_OF_RANGE, _)
    )
}

async fn fetch_block_at<R>(
    reader: &R,
    height: L1Height,
    policy: FetchRetryPolicy,
) -> Result<L1BlockData, FetchError>
where
    R: FetchReader + ?Sized,
{
    let mut retry_count = 0;
    let mut delay_ms = policy.backoff().base_delay_ms();

    loop {
        match reader.get_block_at(height).await {
            Ok(block) => {
                let hash = block.block_hash();
                return Ok(L1BlockData::new(height, hash, block));
            }
            Err(source) if is_height_out_of_range(&source) => {
                return Err(FetchError::HeightOutOfRange { height, source });
            }
            Err(source) if matches!(source, ClientError::MaxRetriesExceeded(_)) => {
                return Err(FetchError::RetriesExhausted {
                    height,
                    max_retries: policy.max_retries(),
                    source,
                });
            }
            Err(source) if !is_retryable_fetch_error(&source) => {
                return Err(FetchError::Client { height, source });
            }
            Err(source) if retry_count >= policy.max_retries() => {
                return Err(FetchError::RetriesExhausted {
                    height,
                    max_retries: policy.max_retries(),
                    source,
                });
            }
            Err(_) => {
                retry_count += 1;
                sleep(Duration::from_millis(delay_ms)).await;
                delay_ms = policy.backoff().next_delay_ms(delay_ms);
            }
        }
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
        block_responses: Mutex<HashMap<L1Height, VecDeque<ClientResult<Block>>>>,
        block_calls: Mutex<Vec<L1Height>>,
    }

    impl MockReader {
        fn add_block_responses(
            mut self,
            height: L1Height,
            responses: Vec<ClientResult<Block>>,
        ) -> Self {
            self.block_responses
                .get_mut()
                .expect("poisoned")
                .insert(height, responses.into_iter().collect());
            self
        }

        fn count_block_calls(&self, height: L1Height) -> usize {
            self.block_calls
                .lock()
                .expect("poisoned")
                .iter()
                .filter(|h| **h == height)
                .count()
        }
    }

    impl FetchReader for MockReader {
        fn get_block_at(&self, height: L1Height) -> BoxFuture<'_, ClientResult<Block>> {
            self.block_calls.lock().expect("poisoned").push(height);

            let response = self
                .block_responses
                .lock()
                .expect("poisoned")
                .get_mut(&height)
                .and_then(|queue| queue.pop_front())
                .expect("missing block response for height");

            Box::pin(async move { response })
        }
    }

    fn build_block(nonce: u32) -> Block {
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

    fn build_fetch_policy() -> FetchRetryPolicy {
        FetchRetryPolicy::new(5, ExponentialBackoff::new(0, 150, 100))
    }

    fn fetch_range_with_test_policy<'a, R>(
        reader: &'a R,
        start_height: L1Height,
        end_height: L1Height,
    ) -> FetchRangeResult<'a>
    where
        R: FetchReader + ?Sized,
    {
        fetch_range(reader, start_height, end_height, build_fetch_policy())
    }

    #[test]
    fn test_inverted_range() {
        let reader = MockReader::default();

        let Err(error) = fetch_range_with_test_policy(&reader, 12, 10) else {
            panic!("invalid range must fail");
        };

        assert_eq!(
            error,
            FetchRangeError::Inverted {
                start_height: 12,
                end_height: 10,
            }
        );
    }

    #[test]
    fn test_max_range() {
        let reader = MockReader::default();

        // This test covers synchronous range validation only; consuming the
        // stream would require mock responses for every block in the range.
        let _stream =
            fetch_range_with_test_policy(&reader, 0, (MAX_EXTRACTION_BLOCK_RANGE - 1) as L1Height)
                .expect("range at maximum size must be accepted");
    }

    #[test]
    fn test_oversized_range() {
        let reader = MockReader::default();

        let Err(error) =
            fetch_range_with_test_policy(&reader, 0, MAX_EXTRACTION_BLOCK_RANGE as L1Height)
        else {
            panic!("range above maximum size must fail");
        };

        assert_eq!(
            error,
            FetchRangeError::TooLarge {
                start_height: 0,
                end_height: MAX_EXTRACTION_BLOCK_RANGE as L1Height,
            }
        );
    }

    #[tokio::test]
    async fn test_height_order() {
        let heights: [L1Height; 3] = [10, 11, 12];
        let expected = heights
            .iter()
            .map(|height| {
                let block = build_block(*height);
                (*height, block.block_hash(), block)
            })
            .collect::<Vec<_>>();

        let mut reader = MockReader::default();
        for (height, _, block) in &expected {
            reader = reader.add_block_responses(*height, vec![Ok(block.clone())]);
        }

        let blocks = fetch_range_with_test_policy(&reader, 10, 12)
            .expect("valid range")
            .try_collect::<Vec<L1BlockData>>()
            .await
            .expect("fetch must succeed");

        assert_eq!(blocks.len(), expected.len());
        for (block, (height, hash, _)) in blocks.iter().zip(expected.iter()) {
            assert_eq!(block.height(), *height);
            assert_eq!(block.hash(), *hash);
        }
    }

    #[tokio::test]
    async fn test_retryable_error() {
        let block = build_block(8);
        let hash = block.block_hash();
        let reader = MockReader::default().add_block_responses(
            8,
            vec![
                Err(ClientError::Connection("temporary".to_string())),
                Ok(block),
            ],
        );

        let blocks = fetch_range_with_test_policy(&reader, 8, 8)
            .expect("valid range")
            .try_collect::<Vec<L1BlockData>>()
            .await
            .expect("block fetch must succeed after retry");

        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].hash(), hash);
        assert_eq!(reader.count_block_calls(8), 2);
    }

    #[tokio::test]
    async fn test_terminal_error() {
        let reader = MockReader::default().add_block_responses(
            1,
            vec![Err(ClientError::Status(401, "Unauthorized".to_string()))],
        );

        let error = fetch_range_with_test_policy(&reader, 1, 1)
            .expect("valid range")
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
        assert_eq!(reader.count_block_calls(1), 1);
    }

    #[tokio::test]
    async fn test_missing_height() {
        let reader = MockReader::default().add_block_responses(
            42,
            vec![Err(ClientError::Server(
                -8,
                "Block height out of range".to_string(),
            ))],
        );

        let error = fetch_range_with_test_policy(&reader, 42, 42)
            .expect("valid range")
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
        assert_eq!(reader.count_block_calls(42), 1);
    }

    #[tokio::test]
    async fn test_retry_exhaustion() {
        let responses = (0..16)
            .map(|_| Err(ClientError::Connection("temporary".to_string())))
            .collect::<Vec<_>>();
        let reader = MockReader::default().add_block_responses(9, responses);

        let error = fetch_range_with_test_policy(&reader, 9, 9)
            .expect("valid range")
            .try_collect::<Vec<L1BlockData>>()
            .await
            .expect_err("persistent retryable errors must exhaust retries");

        assert!(matches!(
            error,
            FetchError::RetriesExhausted {
                max_retries: 5,
                source: ClientError::Connection(_),
                ..
            }
        ));
    }

    #[tokio::test]
    async fn test_client_retry_exhaustion() {
        let reader = MockReader::default()
            .add_block_responses(100, vec![Err(ClientError::MaxRetriesExceeded(3))]);

        let error = fetch_range_with_test_policy(&reader, 100, 100)
            .expect("valid range")
            .try_collect::<Vec<L1BlockData>>()
            .await
            .expect_err("client max-retries error must map to retries exhausted");

        assert!(matches!(
            error,
            FetchError::RetriesExhausted {
                max_retries: 5,
                source: ClientError::MaxRetriesExceeded(3),
                ..
            }
        ));
    }
}
