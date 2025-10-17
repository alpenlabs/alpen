use async_trait::async_trait;
use eyre::eyre;
use strata_identifiers::{Buf32, OLBlockCommitment, OLBlockId};
use strata_snark_acct_types::UpdateOperationUnconditionalData;

#[derive(Debug)]
pub(crate) struct OlChainStatus {
    pub latest: OLBlockCommitment,
    pub confirmed: OLBlockCommitment,
    pub finalized: OLBlockCommitment,
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
    async fn chain_status(&self) -> eyre::Result<OlChainStatus>;

    /// Retrieves block commitments for a range of slots.
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
    ) -> eyre::Result<Vec<OLBlockCommitment>>;

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
    ) -> eyre::Result<Vec<Vec<UpdateOperationUnconditionalData>>>;
}

/// Retrieves block commitments for a range of slots with validation.
///
/// This is a checked version of `block_commitments_in_range` that validates:
/// - The end slot is greater than the start slot
/// - The number of returned blocks matches the expected count
pub(crate) async fn block_commitments_in_range_checked(
    client: &impl OlClient,
    start_slot: u64,
    end_slot: u64,
) -> eyre::Result<Vec<OLBlockCommitment>> {
    if end_slot <= start_slot {
        return Err(eyre!(
            "block_commitments_in_range; invalid input: end_slot <= start_slot"
        ));
    }
    let blocks = client
        .block_commitments_in_range(start_slot, end_slot)
        .await?;
    let expected_result_len = end_slot - start_slot + 1;
    if blocks.len() != expected_result_len as usize {
        return Err(eyre!(
            "block_commitments_in_range; invalid response: expected_len = {}, got = {}",
            expected_result_len,
            blocks.len()
        ));
    }
    Ok(blocks)
}

/// Retrieves update operations for the specified blocks with validation.
///
/// This is a checked version of `get_update_operations_for_blocks` that validates
/// the number of returned operation vectors matches the number of input blocks.
pub(crate) async fn get_update_operations_for_blocks_checked(
    client: &impl OlClient,
    blocks: Vec<OLBlockId>,
) -> eyre::Result<Vec<Vec<UpdateOperationUnconditionalData>>> {
    let expected_len = blocks.len();
    let res = client.get_update_operations_for_blocks(blocks).await?;
    if res.len() != expected_len {
        return Err(eyre!(
            "get_update_operations_for_blocks; invalid response: expected_len = {}, got = {}",
            expected_len,
            res.len()
        ));
    }

    Ok(res)
}

#[derive(Debug, Default)]
pub(crate) struct DummyOlClient {}

#[async_trait]
impl OlClient for DummyOlClient {
    async fn chain_status(&self) -> eyre::Result<OlChainStatus> {
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
    ) -> eyre::Result<Vec<OLBlockCommitment>> {
        if end_slot < start_slot {
            return Err(eyre!("invalid"));
        }

        Ok((start_slot..=end_slot)
            .map(slot_to_block_commitment)
            .collect())
    }

    async fn get_update_operations_for_blocks(
        &self,
        blocks: Vec<OLBlockId>,
    ) -> eyre::Result<Vec<Vec<UpdateOperationUnconditionalData>>> {
        Ok((blocks.iter().map(|_| vec![])).collect())
    }
}

fn slot_to_block_commitment(slot: u64) -> OLBlockCommitment {
    OLBlockCommitment::new(slot, Buf32::new(u64_to_256(slot)).into())
}

fn u64_to_256(v: u64) -> [u8; 32] {
    unsafe { std::mem::transmute([0, 0, 0, v]) }
}
