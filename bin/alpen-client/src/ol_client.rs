use alpen_ee_common::{OLChainStatus, OLClient, OLClientError, OLEpochSummary};
use async_trait::async_trait;
use strata_identifiers::{Buf32, Epoch, OLBlockCommitment};
use strata_primitives::EpochCommitment;

#[derive(Debug, Default)]
pub(crate) struct DummyOLClient {}

#[async_trait]
impl OLClient for DummyOLClient {
    async fn chain_status(&self) -> Result<OLChainStatus, OLClientError> {
        Ok(OLChainStatus {
            latest: OLBlockCommitment::null(),
            confirmed: EpochCommitment::null(),
            finalized: EpochCommitment::null(),
        })
    }

    async fn epoch_summary(&self, epoch: Epoch) -> Result<OLEpochSummary, OLClientError> {
        let commitment = EpochCommitment::new(
            epoch,
            epoch as u64,
            slot_to_block_commitment(epoch as u64).blkid,
        );
        Ok(OLEpochSummary::new(commitment, commitment, vec![]))
    }
}

fn slot_to_block_commitment(slot: u64) -> OLBlockCommitment {
    OLBlockCommitment::new(slot, Buf32::new(u64_to_256(slot)).into())
}

fn u64_to_256(v: u64) -> [u8; 32] {
    unsafe { std::mem::transmute([0, 0, 0, v]) }
}
