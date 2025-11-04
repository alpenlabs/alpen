use async_trait::async_trait;
use strata_identifiers::{Buf32, OLBlockCommitment, OLBlockId};
use strata_snark_acct_types::UpdateOperationUnconditionalData;

use super::error::OlClientError;

#[derive(Debug)]
pub(crate) struct OlChainStatus {
    pub(crate) latest: OLBlockCommitment,
    pub(crate) confirmed: OLBlockCommitment,
    pub(crate) finalized: OLBlockCommitment,
}

impl OlChainStatus {
    pub(crate) fn latest(&self) -> &OLBlockCommitment {
        &self.latest
    }
    pub(crate) fn confirmed(&self) -> &OLBlockCommitment {
        &self.confirmed
    }
    pub(crate) fn finalized(&self) -> &OLBlockCommitment {
        &self.finalized
    }
}

/// Client interface for interacting with the OL chain.
///
/// Provides methods to view OL Chain data required by an alpen EE fullnode.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub(crate) trait OlClient: Sized + Send + Sync {
    /// Returns the current status of the OL chain.
    ///
    /// Includes the latest, confirmed, and finalized block commitments.
    async fn chain_status(&self) -> Result<OlChainStatus, OlClientError>;

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
    ) -> Result<Vec<OLBlockCommitment>, OlClientError>;

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
    ) -> Result<Vec<Vec<UpdateOperationUnconditionalData>>, OlClientError>;
}

/// Returns the current status of the OL chain.
///
/// This is a checked version of [`OlClient::chain_status`] that validates
/// the slot numbers of latest >= confirmed >= finalized
pub(crate) async fn chain_status_checked(
    client: &impl OlClient,
) -> Result<OlChainStatus, OlClientError> {
    let status = client.chain_status().await?;
    if status.finalized.slot() > status.confirmed.slot()
        || status.confirmed.slot() > status.latest.slot()
    {
        return Err(OlClientError::InvalidChainStatusSlotOrder {
            latest: status.latest.slot(),
            confirmed: status.confirmed.slot(),
            finalized: status.finalized.slot(),
        });
    }
    Ok(status)
}

/// Retrieves block commitments for a range of slots with validation.
///
/// This is a checked version of [`OlClient::block_commitments_in_range`] that validates:
/// - The end slot is greater than the start slot
/// - The number of returned blocks matches the expected count
pub(crate) async fn block_commitments_in_range_checked(
    client: &impl OlClient,
    start_slot: u64,
    end_slot: u64,
) -> Result<Vec<OLBlockCommitment>, OlClientError> {
    if end_slot <= start_slot {
        return Err(OlClientError::InvalidSlotRange {
            start_slot,
            end_slot,
        });
    }
    let blocks = client
        .block_commitments_in_range(start_slot, end_slot)
        .await?;
    let expected_result_len = end_slot - start_slot + 1;
    if blocks.len() != expected_result_len as usize {
        return Err(OlClientError::UnexpectedBlockCount {
            expected: expected_result_len as usize,
            actual: blocks.len(),
        });
    }
    Ok(blocks)
}

/// Retrieves update operations for the specified blocks with validation.
///
/// This is a checked version of [`OlClient::get_update_operations_for_blocks`] that validates
/// the number of returned operation vectors matches the number of input blocks.
pub(crate) async fn get_update_operations_for_blocks_checked(
    client: &impl OlClient,
    blocks: Vec<OLBlockId>,
) -> Result<Vec<Vec<UpdateOperationUnconditionalData>>, OlClientError> {
    let expected_len = blocks.len();
    let res = client.get_update_operations_for_blocks(blocks).await?;
    if res.len() != expected_len {
        return Err(OlClientError::UnexpectedOperationCount {
            expected: expected_len,
            actual: res.len(),
        });
    }

    Ok(res)
}

#[derive(Debug, Default)]
pub(crate) struct DummyOlClient {}

#[async_trait]
impl OlClient for DummyOlClient {
    async fn chain_status(&self) -> Result<OlChainStatus, OlClientError> {
        Ok(OlChainStatus {
            latest: OLBlockCommitment::null(),
            confirmed: OLBlockCommitment::null(),
            finalized: OLBlockCommitment::null(),
        })
    }

    async fn block_commitments_in_range(
        &self,
        start_slot: u64,
        end_slot: u64,
    ) -> Result<Vec<OLBlockCommitment>, OlClientError> {
        if end_slot < start_slot {
            return Err(OlClientError::InvalidSlotRange {
                start_slot,
                end_slot,
            });
        }

        Ok((start_slot..=end_slot)
            .map(slot_to_block_commitment)
            .collect())
    }

    async fn get_update_operations_for_blocks(
        &self,
        blocks: Vec<OLBlockId>,
    ) -> Result<Vec<Vec<UpdateOperationUnconditionalData>>, OlClientError> {
        Ok((blocks.iter().map(|_| vec![])).collect())
    }
}

fn slot_to_block_commitment(slot: u64) -> OLBlockCommitment {
    OLBlockCommitment::new(slot, Buf32::new(u64_to_256(slot)).into())
}

fn u64_to_256(v: u64) -> [u8; 32] {
    unsafe { std::mem::transmute([0, 0, 0, v]) }
}

#[cfg(test)]
mod tests {
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
            let mut mock_client = MockOlClient::new();

            // Should not call the underlying method if validation fails
            mock_client.expect_block_commitments_in_range().times(0);

            let result = block_commitments_in_range_checked(&mock_client, 100, 100).await;

            assert!(result.is_err());
            assert!(matches!(
                result.unwrap_err(),
                OlClientError::InvalidSlotRange { .. }
            ));
        }

        #[tokio::test]
        async fn test_validates_end_less_than_start() {
            let mut mock_client = MockOlClient::new();

            mock_client.expect_block_commitments_in_range().times(0);

            let result = block_commitments_in_range_checked(&mock_client, 100, 50).await;

            assert!(result.is_err());
            assert!(matches!(
                result.unwrap_err(),
                OlClientError::InvalidSlotRange { .. }
            ));
        }

        #[tokio::test]
        async fn test_validates_result_length_matches_expected() {
            let mut mock_client = MockOlClient::new();

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
                OlClientError::UnexpectedBlockCount { expected, actual } => {
                    assert_eq!(expected, 6);
                    assert_eq!(actual, 3);
                }
                _ => panic!("Expected UnexpectedBlockCount error"),
            }
        }

        #[tokio::test]
        async fn test_success_with_single_block() {
            let mut mock_client = MockOlClient::new();

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
            let mut mock_client = MockOlClient::new();

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
            let mut mock_client = MockOlClient::new();

            mock_client
                .expect_block_commitments_in_range()
                .times(1)
                .returning(|_, _| Err(OlClientError::network("network error")));

            let result = block_commitments_in_range_checked(&mock_client, 100, 105).await;

            assert!(result.is_err());
            assert!(matches!(result.unwrap_err(), OlClientError::Network(_)));
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
            let mut mock_client = MockOlClient::new();

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
                OlClientError::UnexpectedOperationCount { expected, actual } => {
                    assert_eq!(expected, 3);
                    assert_eq!(actual, 2);
                }
                _ => panic!("Expected UnexpectedOperationCount error"),
            }
        }

        #[tokio::test]
        async fn test_success_with_empty_blocks() {
            let mut mock_client = MockOlClient::new();

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
            let mut mock_client = MockOlClient::new();

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
            let mut mock_client = MockOlClient::new();

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
            let mut mock_client = MockOlClient::new();

            let block_ids = vec![make_block_id(1), make_block_id(2)];

            mock_client
                .expect_get_update_operations_for_blocks()
                .times(1)
                .returning(|_| Err(OlClientError::rpc("rpc error")));

            let result = get_update_operations_for_blocks_checked(&mock_client, block_ids).await;

            assert!(result.is_err());
            assert!(matches!(result.unwrap_err(), OlClientError::Rpc(_)));
        }
    }
}
