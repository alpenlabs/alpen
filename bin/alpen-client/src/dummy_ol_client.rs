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

    async fn account_genesis_epoch(&self) -> Result<EpochCommitment, OLClientError> {
        Ok(self.genesis_epoch)
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

    async fn get_l1_header_commitment(&self, l1_height: u64) -> Result<Hash, OLClientError> {
        Ok(Hash::from(u64_to_256(l1_height)))
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

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        #[test]
        fn u64_to_256_is_deterministic_and_reversible(v: u64) {
            let bytes = u64_to_256(v);

            // Verify structure: [1u64 LE][0u64][0u64][v LE]
            let prefix = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
            let mid1 = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
            let mid2 = u64::from_le_bytes(bytes[16..24].try_into().unwrap());
            let suffix = u64::from_le_bytes(bytes[24..32].try_into().unwrap());

            prop_assert_eq!(prefix, 1u64);
            prop_assert_eq!(mid1, 0u64);
            prop_assert_eq!(mid2, 0u64);
            prop_assert_eq!(suffix, v);
        }

        #[test]
        fn u64_to_256_produces_unique_outputs(v1: u64, v2: u64) {
            prop_assume!(v1 != v2);
            let bytes1 = u64_to_256(v1);
            let bytes2 = u64_to_256(v2);
            prop_assert_ne!(bytes1, bytes2);
        }
    }
}
