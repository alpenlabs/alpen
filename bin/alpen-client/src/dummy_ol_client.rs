// TODO: remove this "dummy" implementation once OL RPCs are ready and `RpcOLCLient`` can be used
// instead.
use alpen_ee_common::{
    OLBlockData, OLChainStatus, OLClient, OLClientError, OLEpochSummary, SequencerOLClient,
};
use async_trait::async_trait;
use strata_identifiers::{Buf32, Epoch, OLBlockCommitment};
use strata_primitives::EpochCommitment;
use strata_snark_acct_types::SnarkAccountUpdate;

#[derive(Debug)]
pub(crate) struct DummyOLClient {
    pub(crate) genesis_epoch: EpochCommitment,
}

#[async_trait]
impl OLClient for DummyOLClient {
    async fn chain_status(&self) -> Result<OLChainStatus, OLClientError> {
        Ok(OLChainStatus {
            latest: self.genesis_epoch.to_block_commitment(),
            confirmed: self.genesis_epoch,
            finalized: self.genesis_epoch,
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

#[async_trait]
impl SequencerOLClient for DummyOLClient {
    async fn chain_status(&self) -> Result<OLChainStatus, OLClientError> {
        <Self as OLClient>::chain_status(self).await
    }

    async fn get_inbox_messages(
        &self,
        min_slot: u64,
        max_slot: u64,
    ) -> Result<Vec<OLBlockData>, OLClientError> {
        let mut blocks = Vec::with_capacity((max_slot - min_slot + 1) as usize);
        for slot in min_slot..=max_slot {
            let commitment = slot_to_block_commitment(slot);
            blocks.push(OLBlockData {
                commitment,
                inbox_messages: vec![],
                next_inbox_msg_idx: 0,
            })
        }
        Ok(blocks)
    }

    async fn submit_update(&self, _update: SnarkAccountUpdate) -> Result<(), OLClientError> {
        Ok(())
    }
}

fn slot_to_block_commitment(slot: u64) -> OLBlockCommitment {
    OLBlockCommitment::new(slot, Buf32::new(u64_to_256(slot)).into())
}

fn u64_to_256(v: u64) -> [u8; 32] {
    unsafe { std::mem::transmute([1, 0, 0, v]) }
}
