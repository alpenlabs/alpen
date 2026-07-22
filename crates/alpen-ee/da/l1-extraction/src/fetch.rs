//! Streams Bitcoin blocks from bitcoind with retry.

use std::{future::Future, num::NonZeroU64};

use bitcoin::Block;
use bitcoind_async_client::{error::ClientError, traits::Reader, ClientResult};
use futures::{stream, Stream};
use strata_btc_types::BlockHashExt;
use strata_common::retry::{policies::ExponentialBackoff, retry_with_backoff_async};
use strata_identifiers::{L1BlockCommitment, L1Height};
use thiserror::Error;

/// bitcoind RPC error code for "Block height out of range".
const BITCOIND_BLOCK_HEIGHT_OUT_OF_RANGE: i32 = -8;

/// Raw Bitcoin block data paired with its fetched L1 commitment.
#[derive(Debug, Clone)]
pub struct L1BlockData {
    commitment: L1BlockCommitment,
    block: Block,
}

impl L1BlockData {
    /// Creates fetched L1 block data.
    pub fn new(height: L1Height, block: Block) -> Self {
        let commitment = L1BlockCommitment::new(height, block.block_hash().to_l1_block_id());
        Self { commitment, block }
    }

    /// Returns the L1 block commitment where the block was fetched.
    pub fn commitment(&self) -> L1BlockCommitment {
        self.commitment
    }

    /// Returns the L1 height where the block was fetched.
    pub fn height(&self) -> L1Height {
        self.commitment.height()
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
        "extraction range too large: requested {requested_block_count}, max {max_block_count}"
    )]
    TooLarge {
        requested_block_count: u64,
        max_block_count: NonZeroU64,
    },
}

/// Retry and range policy for bounded L1 block fetches.
#[derive(Debug)]
pub struct FetchRetryPolicy {
    max_retries: u16,
    backoff: ExponentialBackoff,
    max_block_count: NonZeroU64,
}

impl FetchRetryPolicy {
    /// Creates a fetch retry policy.
    pub fn new(max_retries: u16, backoff: ExponentialBackoff, max_block_count: NonZeroU64) -> Self {
        Self {
            max_retries,
            backoff,
            max_block_count,
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

    /// Returns the maximum inclusive block count accepted by one fetch range.
    pub fn max_block_count(&self) -> NonZeroU64 {
        self.max_block_count
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
}

/// Narrow adapter seam over the bitcoind block reader method used by fetch.
///
/// This trait exists to unit-test fetch retry behavior without depending on a
/// live bitcoind instance. It is not intended as a broad extension API.
pub trait FetchReader: Send + Sync {
    /// Fetches the block at `height`.
    fn get_block_at(
        &self,
        height: L1Height,
    ) -> impl Future<Output = ClientResult<Block>> + Send + '_;
}

impl<T> FetchReader for T
where
    T: Reader + Send + Sync,
{
    fn get_block_at(
        &self,
        height: L1Height,
    ) -> impl Future<Output = ClientResult<Block>> + Send + '_ {
        Reader::get_block_at(self, u64::from(height))
    }
}

/// Returns an ordered stream of Bitcoin blocks using the supplied retry policy.
pub fn fetch_range<'a, R>(
    reader: &'a R,
    start_height: L1Height,
    end_height: L1Height,
    policy: FetchRetryPolicy,
) -> Result<impl Stream<Item = Result<L1BlockData, FetchError>> + 'a, FetchRangeError>
where
    R: FetchReader,
{
    if start_height > end_height {
        return Err(FetchRangeError::Inverted {
            start_height,
            end_height,
        });
    }

    let requested_block_count = u64::from(end_height - start_height) + 1;
    if requested_block_count > policy.max_block_count().get() {
        return Err(FetchRangeError::TooLarge {
            requested_block_count,
            max_block_count: policy.max_block_count(),
        });
    }

    Ok(stream::unfold(
        (Some(start_height), policy),
        move |(next_height, policy)| async move {
            let height = next_height?;
            let next_height = (height != end_height).then_some(height + 1);
            let result = fetch_block_at(reader, height, &policy).await;
            Some((result, (next_height, policy)))
        },
    ))
}

fn convert_client_error_to_fetch_error(
    height: L1Height,
    max_retries: u16,
    source: ClientError,
) -> FetchError {
    match source {
        source @ ClientError::Server(code, _) if code == BITCOIND_BLOCK_HEIGHT_OUT_OF_RANGE => {
            FetchError::HeightOutOfRange { height, source }
        }
        source => FetchError::RetriesExhausted {
            height,
            max_retries,
            source,
        },
    }
}

async fn fetch_block_at<R>(
    reader: &R,
    height: L1Height,
    policy: &FetchRetryPolicy,
) -> Result<L1BlockData, FetchError>
where
    R: FetchReader,
{
    let block = retry_with_backoff_async(
        "l1_fetch_block",
        policy.max_retries(),
        policy.backoff(),
        || reader.get_block_at(height),
    )
    .await
    .map_err(|source| convert_client_error_to_fetch_error(height, policy.max_retries(), source))?;

    Ok(L1BlockData::new(height, block))
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{HashMap, VecDeque},
        iter::repeat_with,
        num::NonZeroU64,
        sync::Mutex,
    };

    use bitcoin::{
        block::{Header, Version},
        hashes::Hash,
        Block, BlockHash, TxMerkleNode,
    };
    use bitcoind_async_client::error::ClientError;
    use futures::TryStreamExt;
    use strata_btc_types::BlockHashExt;

    use super::*;

    const TEST_MAX_BLOCK_COUNT: u64 = 2_000;

    #[derive(Default, Debug)]
    struct MockReader {
        block_responses: Mutex<HashMap<L1Height, VecDeque<ClientResult<Block>>>>,
        block_calls: Mutex<Vec<L1Height>>,
    }

    impl MockReader {
        fn add_block_responses(
            self,
            height: L1Height,
            responses: Vec<ClientResult<Block>>,
        ) -> Self {
            self.block_responses
                .lock()
                .expect("block responses lock")
                .insert(height, responses.into());
            self
        }

        fn block_calls(&self) -> Vec<L1Height> {
            self.block_calls.lock().expect("block calls lock").clone()
        }
    }

    impl FetchReader for MockReader {
        async fn get_block_at(&self, height: L1Height) -> ClientResult<Block> {
            self.block_calls
                .lock()
                .expect("block calls lock")
                .push(height);
            self.block_responses
                .lock()
                .expect("block responses lock")
                .get_mut(&height)
                .and_then(VecDeque::pop_front)
                .unwrap_or_else(|| Err(ClientError::Server(-8, "missing height".into())))
        }
    }

    fn build_fetch_policy() -> FetchRetryPolicy {
        FetchRetryPolicy::new(
            5,
            ExponentialBackoff::new(0, 150, 100),
            NonZeroU64::new(TEST_MAX_BLOCK_COUNT).expect("non-zero max block count"),
        )
    }

    fn fetch_range_with_test_policy<'a>(
        reader: &'a MockReader,
        start_height: L1Height,
        end_height: L1Height,
    ) -> Result<impl Stream<Item = Result<L1BlockData, FetchError>> + 'a, FetchRangeError> {
        fetch_range(reader, start_height, end_height, build_fetch_policy())
    }

    fn build_block_with_prev_hash(hash: BlockHash) -> Block {
        Block {
            header: Header {
                version: Version::from_consensus(1),
                prev_blockhash: hash,
                merkle_root: TxMerkleNode::all_zeros(),
                time: 0,
                bits: Default::default(),
                nonce: 0,
            },
            txdata: Vec::new(),
        }
    }

    fn make_block_hash(seed: u8) -> BlockHash {
        BlockHash::from_byte_array([seed; 32])
    }

    #[test]
    fn test_inverted_range() {
        let reader = MockReader::default();
        let err = match fetch_range_with_test_policy(&reader, 2, 1) {
            Ok(_) => panic!("range must reject"),
            Err(err) => err,
        };
        assert!(matches!(
            err,
            FetchRangeError::Inverted {
                start_height: 2,
                end_height: 1,
            }
        ));
    }

    #[test]
    fn test_max_range() {
        let reader = MockReader::default();
        let stream =
            fetch_range_with_test_policy(&reader, 0, (TEST_MAX_BLOCK_COUNT - 1) as L1Height)
                .expect("max range accepted");
        drop(stream);
    }

    #[test]
    fn test_oversized_range() {
        let reader = MockReader::default();
        let err = match fetch_range_with_test_policy(&reader, 0, TEST_MAX_BLOCK_COUNT as L1Height) {
            Ok(_) => panic!("oversized range must reject"),
            Err(err) => err,
        };
        assert!(matches!(
            err,
            FetchRangeError::TooLarge {
                requested_block_count,
                max_block_count,
            } if requested_block_count == TEST_MAX_BLOCK_COUNT + 1
                && max_block_count.get() == TEST_MAX_BLOCK_COUNT
        ));
    }

    #[tokio::test]
    async fn test_height_order() {
        let heights = [10, 11, 12];
        let mut reader = MockReader::default();
        let expected = heights
            .iter()
            .map(|height| {
                let block = build_block_with_prev_hash(make_block_hash(*height as u8));
                (*height, block.block_hash().to_l1_block_id(), block)
            })
            .collect::<Vec<_>>();
        for (height, _, block) in &expected {
            reader = reader.add_block_responses(*height, vec![Ok(block.clone())]);
        }

        let blocks = fetch_range_with_test_policy(&reader, 10, 12)
            .expect("stream builds")
            .try_collect::<Vec<_>>()
            .await
            .expect("fetch succeeds");

        assert_eq!(reader.block_calls(), heights);
        for (block, (height, expected_block_id, _)) in blocks.iter().zip(expected.iter()) {
            assert_eq!(block.height(), *height);
            assert_eq!(block.commitment().blkid(), expected_block_id);
        }
    }

    #[tokio::test]
    async fn test_retryable_error() {
        let block = build_block_with_prev_hash(make_block_hash(7));
        let expected_block_id = block.block_hash().to_l1_block_id();
        let reader = MockReader::default().add_block_responses(
            100,
            vec![
                Err(ClientError::Connection("connection reset".into())),
                Ok(block),
            ],
        );

        let blocks = fetch_range_with_test_policy(&reader, 100, 100)
            .expect("stream builds")
            .try_collect::<Vec<_>>()
            .await
            .expect("retry succeeds");

        assert_eq!(blocks[0].commitment().blkid(), &expected_block_id);
        assert_eq!(reader.block_calls(), vec![100, 100]);
    }

    #[tokio::test]
    async fn test_non_retryable_error_exhausts_retries() {
        let reader = MockReader::default().add_block_responses(
            100,
            repeat_with(|| Err(ClientError::ReqBuilder("bad request".into())))
                .take(6)
                .collect(),
        );

        let err = fetch_range_with_test_policy(&reader, 100, 100)
            .expect("stream builds")
            .try_collect::<Vec<_>>()
            .await
            .expect_err("client error must fail");

        assert!(matches!(
            err,
            FetchError::RetriesExhausted {
                height: 100,
                max_retries: 5,
                source: ClientError::ReqBuilder(_),
            }
        ));
        assert_eq!(reader.block_calls(), vec![100; 6]);
    }

    #[tokio::test]
    async fn test_missing_height() {
        let reader = MockReader::default().add_block_responses(
            50,
            repeat_with(|| Err(ClientError::Server(-8, "missing".into())))
                .take(6)
                .collect(),
        );

        let err = fetch_range_with_test_policy(&reader, 50, 50)
            .expect("stream builds")
            .try_collect::<Vec<_>>()
            .await
            .expect_err("height out of range must fail");

        assert!(matches!(
            err,
            FetchError::HeightOutOfRange {
                height: 50,
                source: ClientError::Server(-8, _),
            }
        ));
    }

    #[tokio::test]
    async fn test_retry_exhaustion() {
        let reader = MockReader::default().add_block_responses(
            100,
            repeat_with(|| Err(ClientError::Connection("down".into())))
                .take(6)
                .collect(),
        );

        let err = fetch_range_with_test_policy(&reader, 100, 100)
            .expect("stream builds")
            .try_collect::<Vec<_>>()
            .await
            .expect_err("retry exhaustion must fail");

        assert!(matches!(
            err,
            FetchError::RetriesExhausted {
                height: 100,
                max_retries: 5,
                source: ClientError::Connection(_),
            }
        ));
        assert_eq!(reader.block_calls(), vec![100; 6]);
    }

    #[tokio::test]
    async fn test_client_retry_exhaustion() {
        let reader = MockReader::default()
            .add_block_responses(100, vec![Err(ClientError::MaxRetriesExceeded(3)); 6]);

        let err = fetch_range_with_test_policy(&reader, 100, 100)
            .expect("stream builds")
            .try_collect::<Vec<_>>()
            .await
            .expect_err("client retry exhaustion must fail");

        assert!(matches!(
            err,
            FetchError::RetriesExhausted {
                height: 100,
                max_retries: 5,
                source: ClientError::MaxRetriesExceeded(3),
            }
        ));
    }
}
