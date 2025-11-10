use alpen_ee_common::{OlChainStatus, OlClient, OlClientError};
use async_trait::async_trait;
use strata_identifiers::{Buf32, OLBlockCommitment, OLBlockId};
use strata_snark_acct_types::UpdateInputData;

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
    ) -> Result<Vec<Vec<UpdateInputData>>, OlClientError> {
        Ok((blocks.iter().map(|_| vec![])).collect())
    }
}

fn slot_to_block_commitment(slot: u64) -> OLBlockCommitment {
    OLBlockCommitment::new(slot, Buf32::new(u64_to_256(slot)).into())
}

fn u64_to_256(v: u64) -> [u8; 32] {
    unsafe { std::mem::transmute([0, 0, 0, v]) }
}
