//! Dummy OL client for testing EE functionality without a real OL node.
//!
//! This module provides a mock implementation of the OL client traits that returns
//! minimal valid responses. It's useful for testing EE-specific functionality
//! in isolation without needing to run a full OL node.

use alpen_ee_common::{
    OLAccountStateView, OLBlockData, OLChainStatus, OLClient, OLClientError, OLEpochSummary,
    SequencerOLClient,
};
use async_trait::async_trait;
use strata_acct_types::Hash;
use strata_identifiers::{Buf32, Epoch, OLBlockCommitment};
use strata_primitives::EpochCommitment;
use strata_snark_acct_types::{ProofState, Seqno, SnarkAccountUpdate};

/// A dummy OL client that returns mock responses for testing.
///
/// This client does not communicate with any real OL node. Instead, it returns
/// minimal valid responses that allow the EE to function in isolation.
#[derive(Debug)]
pub(crate) struct DummyOLClient {
    pub(crate) genesis_epoch: EpochCommitment,
}

impl DummyOLClient {
    fn slot_to_block_commitment(&self, slot: u64) -> OLBlockCommitment {
        if slot == self.genesis_epoch.last_slot() {
            self.genesis_epoch.to_block_commitment()
        } else {
            slot_to_block_commitment(slot)
        }
    }
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
            self.slot_to_block_commitment(epoch as u64).blkid,
        );
        // Compute previous epoch commitment for proper chaining.
        // For epoch 0, use genesis; otherwise use epoch - 1.
        let prev = if epoch == 0 {
            self.genesis_epoch
        } else {
            let prev_epoch = epoch - 1;
            EpochCommitment::new(
                prev_epoch,
                prev_epoch as u64,
                self.slot_to_block_commitment(prev_epoch as u64).blkid,
            )
        };
        Ok(OLEpochSummary::new(commitment, prev, vec![]))
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
            let commitment = self.slot_to_block_commitment(slot);
            blocks.push(OLBlockData {
                commitment,
                inbox_messages: vec![],
                next_inbox_msg_idx: 0,
            })
        }
        Ok(blocks)
    }

    async fn get_latest_account_state(&self) -> Result<OLAccountStateView, OLClientError> {
        let proof_state = ProofState::new(Hash::zero(), 0);
        let seq_no = Seqno::zero();
        Ok(OLAccountStateView {
            seq_no,
            proof_state,
        })
    }

    async fn submit_update(&self, _update: SnarkAccountUpdate) -> Result<(), OLClientError> {
        Ok(())
    }
}

fn slot_to_block_commitment(slot: u64) -> OLBlockCommitment {
    OLBlockCommitment::new(slot, Buf32::new(u64_to_256(slot)).into())
}

fn u64_to_256(v: u64) -> [u8; 32] {
    // Use explicit little-endian byte order for deterministic cross-platform behavior.
    let mut result = [0u8; 32];
    result[0..8].copy_from_slice(&1u64.to_le_bytes());
    // bytes 8..16 and 16..24 are already zero
    result[24..32].copy_from_slice(&v.to_le_bytes());
    result
}
