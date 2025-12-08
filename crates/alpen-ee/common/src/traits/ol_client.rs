use async_trait::async_trait;
use strata_identifiers::{OLBlockCommitment, OLBlockId};
use strata_snark_acct_types::{MessageEntry, SnarkAccountUpdate, UpdateInputData};
use thiserror::Error;

use crate::OLChainStatus;

/// Client interface for interacting with the OL chain.
///
/// Provides methods to view OL Chain data required by an alpen EE fullnode.
#[cfg_attr(feature = "test-utils", mockall::automock)]
#[async_trait]
pub trait OLClient: Sized + Send + Sync {
    /// Returns the current status of the OL chain.
    ///
    /// Includes the latest, confirmed, and finalized block commitments.
    async fn chain_status(&self) -> Result<OLChainStatus, OLClientError>;

    /// Retrieves block commitments for a range of slots (inclusive).
    ///
    /// # Arguments
    ///
    /// * `start_slot` - The starting slot number (inclusive)
    /// * `end_slot` - The ending slot number (inclusive)
    ///
    /// # Returns
    ///
    /// A vector of block commitments for the specified slot range.
    async fn block_commitments_in_range(
        &self,
        start_slot: u64,
        end_slot: u64,
    ) -> Result<Vec<OLBlockCommitment>, OLClientError>;

    /// Retrieves update operations for the specified blocks.
    ///
    /// # Arguments
    ///
    /// * `blocks` - A vector of block IDs to fetch update operations for
    ///
    /// # Returns
    ///
    /// A vector where each element contains the update operations for the
    /// corresponding block in the input vector.
    async fn get_update_operations_for_blocks(
        &self,
        blocks: Vec<OLBlockId>,
    ) -> Result<Vec<Vec<UpdateInputData>>, OLClientError>;
}

/// Returns the current status of the OL chain.
///
/// This is a checked version of [`OLClient::chain_status`] that validates
/// the slot numbers of latest >= confirmed >= finalized
pub async fn chain_status_checked(client: &impl OLClient) -> Result<OLChainStatus, OLClientError> {
    let status = client.chain_status().await?;
    if status.finalized.slot() > status.confirmed.slot()
        || status.confirmed.slot() > status.latest.slot()
    {
        return Err(OLClientError::InvalidChainStatusSlotOrder {
            latest: status.latest.slot(),
            confirmed: status.confirmed.slot(),
            finalized: status.finalized.slot(),
        });
    }
    Ok(status)
}

/// Retrieves block commitments for a range of slots with validation.
///
/// This is a checked version of [`OLClient::block_commitments_in_range`] that validates:
/// - The end slot is greater than the start slot
/// - The number of returned blocks matches the expected count
pub async fn block_commitments_in_range_checked(
    client: &impl OLClient,
    start_slot: u64,
    end_slot: u64,
) -> Result<Vec<OLBlockCommitment>, OLClientError> {
    if end_slot <= start_slot {
        return Err(OLClientError::InvalidSlotRange {
            start_slot,
            end_slot,
        });
    }
    let blocks = client
        .block_commitments_in_range(start_slot, end_slot)
        .await?;
    let expected_result_len = end_slot - start_slot + 1;
    if blocks.len() != expected_result_len as usize {
        return Err(OLClientError::UnexpectedBlockCount {
            expected: expected_result_len as usize,
            actual: blocks.len(),
        });
    }
    Ok(blocks)
}

/// Retrieves update operations for the specified blocks with validation.
///
/// This is a checked version of [`OLClient::get_update_operations_for_blocks`] that validates
/// the number of returned operation vectors matches the number of input blocks.
pub async fn get_update_operations_for_blocks_checked(
    client: &impl OLClient,
    blocks: Vec<OLBlockId>,
) -> Result<Vec<Vec<UpdateInputData>>, OLClientError> {
    let expected_len = blocks.len();
    let res = client.get_update_operations_for_blocks(blocks).await?;
    if res.len() != expected_len {
        return Err(OLClientError::UnexpectedOperationCount {
            expected: expected_len,
            actual: res.len(),
        });
    }

    Ok(res)
}

#[derive(Debug)]
pub struct OLBlockData {
    pub commitment: OLBlockCommitment,
    pub inbox_messages: Vec<MessageEntry>,
}

#[cfg_attr(feature = "test-utils", mockall::automock)]
#[async_trait]
pub trait SequencerOLClient {
    async fn chain_status(&self) -> Result<OLChainStatus, OLClientError>;

    async fn get_inbox_messages(
        &self,
        min_slot: u64,
        max_slot: u64,
    ) -> Result<Vec<OLBlockData>, OLClientError>;

    async fn submit_update(&self, update: SnarkAccountUpdate) -> Result<(), OLClientError>;
}

pub async fn get_inbox_messages_checked(
    client: &impl SequencerOLClient,
    min_slot: u64,
    max_slot: u64,
) -> Result<Vec<OLBlockData>, OLClientError> {
    if max_slot < min_slot {
        return Err(OLClientError::InvalidSlotRange {
            start_slot: min_slot,
            end_slot: max_slot,
        });
    }

    let expected_len = (max_slot - min_slot + 1) as usize;
    let res = client.get_inbox_messages(min_slot, max_slot).await?;
    if res.len() != expected_len {
        return Err(OLClientError::UnexpectedInboxMessageCount {
            expected: expected_len,
            actual: res.len(),
        });
    }

    Ok(res)
}

/// Errors that can occur when interacting with the OL client.
#[derive(Debug, Error)]
pub enum OLClientError {
    /// End slot is less than or equal to start slot.
    #[error(
        "invalid slot range: end_slot ({end_slot}) must be greater than start_slot ({start_slot})"
    )]
    InvalidSlotRange { start_slot: u64, end_slot: u64 },

    /// Received a different number of blocks than expected.
    #[error("unexpected block count: expected {expected} blocks, got {actual}")]
    UnexpectedBlockCount { expected: usize, actual: usize },

    /// Received a different number of operation lists than expected.
    #[error("unexpected operation count: expected {expected} operation lists, got {actual}")]
    UnexpectedOperationCount { expected: usize, actual: usize },

    /// Received a different number of operation lists than expected.
    #[error("unexpected inbox message count: expected {expected} message lists, got {actual}")]
    UnexpectedInboxMessageCount { expected: usize, actual: usize },

    /// Chain status slots are not in the correct order (latest >= confirmed >= finalized).
    #[error("unexpected chain status slot order: {latest} >= {confirmed} >= {finalized}")]
    InvalidChainStatusSlotOrder {
        latest: u64,
        confirmed: u64,
        finalized: u64,
    },

    /// Network-related error occurred.
    #[error("network error: {0}")]
    Network(String),

    /// RPC call failed.
    #[error("rpc error: {0}")]
    Rpc(String),

    /// Other unspecified error.
    #[error(transparent)]
    Other(#[from] eyre::Error),
}

impl OLClientError {
    /// Creates a network error.
    pub fn network(msg: impl Into<String>) -> Self {
        Self::Network(msg.into())
    }

    /// Creates an RPC error.
    pub fn rpc(msg: impl Into<String>) -> Self {
        Self::Rpc(msg.into())
    }
}

#[cfg(test)]
mod tests {
    use strata_primitives::Buf32;

    use super::*;

    /// Helper to create a block commitment for testing
    fn make_block_commitment(slot: u64, id: u8) -> OLBlockCommitment {
        let mut bytes = [0u8; 32];
        bytes[0] = id;
        OLBlockCommitment::new(slot, Buf32::new(bytes).into())
    }

    mod block_commitments_in_range_checked_tests {
        use super::*;

        #[tokio::test]
        async fn test_validates_end_greater_than_start() {
            let mut mock_client = MockOLClient::new();

            // Should not call the underlying method if validation fails
            mock_client.expect_block_commitments_in_range().times(0);

            let result = block_commitments_in_range_checked(&mock_client, 100, 100).await;

            assert!(result.is_err());
            assert!(matches!(
                result.unwrap_err(),
                OLClientError::InvalidSlotRange { .. }
            ));
        }

        #[tokio::test]
        async fn test_validates_end_less_than_start() {
            let mut mock_client = MockOLClient::new();

            mock_client.expect_block_commitments_in_range().times(0);

            let result = block_commitments_in_range_checked(&mock_client, 100, 50).await;

            assert!(result.is_err());
            assert!(matches!(
                result.unwrap_err(),
                OLClientError::InvalidSlotRange { .. }
            ));
        }

        #[tokio::test]
        async fn test_validates_result_length_matches_expected() {
            let mut mock_client = MockOLClient::new();

            mock_client
                .expect_block_commitments_in_range()
                .times(1)
                .withf(|start, end| *start == 100 && *end == 105)
                .returning(|_, _| {
                    // Return wrong number of blocks (3 instead of 6)
                    Ok(vec![
                        make_block_commitment(100, 1),
                        make_block_commitment(101, 1),
                        make_block_commitment(102, 1),
                    ])
                });

            let result = block_commitments_in_range_checked(&mock_client, 100, 105).await;

            assert!(result.is_err());
            match result.unwrap_err() {
                OLClientError::UnexpectedBlockCount { expected, actual } => {
                    assert_eq!(expected, 6);
                    assert_eq!(actual, 3);
                }
                _ => panic!("Expected UnexpectedBlockCount error"),
            }
        }

        #[tokio::test]
        async fn test_success_with_single_block() {
            let mut mock_client = MockOLClient::new();

            mock_client
                .expect_block_commitments_in_range()
                .times(1)
                .withf(|start, end| *start == 100 && *end == 101)
                .returning(|start, end| {
                    Ok((start..=end)
                        .map(|slot| make_block_commitment(slot, slot as u8))
                        .collect())
                });

            let result = block_commitments_in_range_checked(&mock_client, 100, 101)
                .await
                .unwrap();

            assert_eq!(result.len(), 2);
            assert_eq!(result[0].slot(), 100);
            assert_eq!(result[1].slot(), 101);
        }

        #[tokio::test]
        async fn test_success_with_multiple_blocks() {
            let mut mock_client = MockOLClient::new();

            mock_client
                .expect_block_commitments_in_range()
                .times(1)
                .withf(|start, end| *start == 100 && *end == 110)
                .returning(|start, end| {
                    Ok((start..=end)
                        .map(|slot| make_block_commitment(slot, slot as u8))
                        .collect())
                });

            let result = block_commitments_in_range_checked(&mock_client, 100, 110)
                .await
                .unwrap();

            assert_eq!(result.len(), 11);
            assert_eq!(result[0].slot(), 100);
            assert_eq!(result[10].slot(), 110);
        }

        #[tokio::test]
        async fn test_propagates_client_error() {
            let mut mock_client = MockOLClient::new();

            mock_client
                .expect_block_commitments_in_range()
                .times(1)
                .returning(|_, _| Err(OLClientError::network("network error")));

            let result = block_commitments_in_range_checked(&mock_client, 100, 105).await;

            assert!(result.is_err());
            assert!(matches!(result.unwrap_err(), OLClientError::Network(_)));
        }
    }

    mod get_update_operations_for_blocks_checked_tests {
        use super::*;

        fn make_block_id(id: u8) -> OLBlockId {
            let mut bytes = [0u8; 32];
            bytes[0] = id;
            Buf32::new(bytes).into()
        }

        #[tokio::test]
        async fn test_validates_result_length_matches_input() {
            let mut mock_client = MockOLClient::new();

            let block_ids = vec![make_block_id(1), make_block_id(2), make_block_id(3)];

            mock_client
                .expect_get_update_operations_for_blocks()
                .times(1)
                .returning(|_| {
                    // Return wrong number of operation lists (2 instead of 3)
                    Ok(vec![vec![], vec![]])
                });

            let result = get_update_operations_for_blocks_checked(&mock_client, block_ids).await;

            assert!(result.is_err());
            match result.unwrap_err() {
                OLClientError::UnexpectedOperationCount { expected, actual } => {
                    assert_eq!(expected, 3);
                    assert_eq!(actual, 2);
                }
                _ => panic!("Expected UnexpectedOperationCount error"),
            }
        }

        #[tokio::test]
        async fn test_success_with_empty_blocks() {
            let mut mock_client = MockOLClient::new();

            let block_ids: Vec<OLBlockId> = vec![];

            mock_client
                .expect_get_update_operations_for_blocks()
                .times(1)
                .returning(|_| Ok(vec![]));

            let result = get_update_operations_for_blocks_checked(&mock_client, block_ids.clone())
                .await
                .unwrap();

            assert_eq!(result.len(), 0);
        }

        #[tokio::test]
        async fn test_success_with_single_block() {
            let mut mock_client = MockOLClient::new();

            let block_ids = vec![make_block_id(1)];

            mock_client
                .expect_get_update_operations_for_blocks()
                .times(1)
                .returning(|blocks| Ok(vec![vec![]; blocks.len()]));

            let result = get_update_operations_for_blocks_checked(&mock_client, block_ids.clone())
                .await
                .unwrap();

            assert_eq!(result.len(), 1);
            assert_eq!(result[0].len(), 0);
        }

        #[tokio::test]
        async fn test_success_with_multiple_blocks() {
            let mut mock_client = MockOLClient::new();

            let block_ids = vec![
                make_block_id(1),
                make_block_id(2),
                make_block_id(3),
                make_block_id(4),
                make_block_id(5),
            ];

            mock_client
                .expect_get_update_operations_for_blocks()
                .times(1)
                .returning(|blocks| Ok(vec![vec![]; blocks.len()]));

            let result = get_update_operations_for_blocks_checked(&mock_client, block_ids.clone())
                .await
                .unwrap();

            assert_eq!(result.len(), 5);
        }

        #[tokio::test]
        async fn test_propagates_client_error() {
            let mut mock_client = MockOLClient::new();

            let block_ids = vec![make_block_id(1), make_block_id(2)];

            mock_client
                .expect_get_update_operations_for_blocks()
                .times(1)
                .returning(|_| Err(OLClientError::rpc("rpc error")));

            let result = get_update_operations_for_blocks_checked(&mock_client, block_ids).await;

            assert!(result.is_err());
            assert!(matches!(result.unwrap_err(), OLClientError::Rpc(_)));
        }
    }
}
