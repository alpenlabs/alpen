//! Duty extraction for sequencers with embedded templates.
//!
//! Key improvement: Templates are generated and embedded directly in duties,
//! eliminating the need for separate template fetch requests.

use ssz::Encode;
use strata_checkpoint_types_ssz::CheckpointPayload;
use strata_crypto::hash;
use strata_ol_block_assembly::FullBlockTemplate;
use strata_ol_chain_types_new::Epoch;
use strata_primitives::{Buf32, OLBlockId};

use crate::types::BlockTemplateExt;

/// Describes when we'll stop working to fulfill a duty.
#[derive(Clone, Debug)]
pub enum Expiry {
    /// Duty expires when we see the next block.
    NextBlock,

    /// Duty expires when block is finalized to L1 in a batch.
    BlockFinalized,

    /// Duty expires after a certain timestamp.
    Timestamp(u64),

    /// Duty expires after a specific L2 block is finalized
    BlockIdFinalized(OLBlockId),

    /// Duty expires after a specific checkpoint is finalized on bitcoin
    CheckpointFinalized(Epoch),
}

#[derive(Clone, Debug)]
pub enum Duty {
    /// Duty to sign block
    SignBlock(BlockSigningDuty),

    /// Duty to sign checkpoint
    SignCheckpoint(CheckpointSigningDuty),
}

impl Duty {
    /// Expiry of the duty
    pub fn expiry(&self) -> Expiry {
        match self {
            Self::SignBlock(_) => Expiry::NextBlock,
            Self::SignCheckpoint(d) => Expiry::CheckpointFinalized(d.checkpoint.new_tip().epoch),
        }
    }

    /// Unique identifier for the duty
    pub fn generate_id(&self) -> Buf32 {
        match self {
            Self::SignBlock(b) => b.template_id().into(),
            Self::SignCheckpoint(c) => {
                let encoded = c.checkpoint.as_ssz_bytes();
                hash::raw(&encoded)
            }
        }
    }
}

/// A duty to sign a block with an embedded template.
#[derive(Debug, Clone)]
pub struct BlockSigningDuty {
    /// The block template to sign.
    pub template: FullBlockTemplate,
}

/// A duty to sign a checkpoint.
#[derive(Debug, Clone)]
pub struct CheckpointSigningDuty {
    /// The checkpoint to sign.
    pub checkpoint: CheckpointPayload,
}

impl BlockSigningDuty {
    /// Returns the template ID.
    pub fn template_id(&self) -> OLBlockId {
        self.template.template_id()
    }

    pub fn target_timestamp(&self) -> u64 {
        self.template.timestamp()
    }

    /// Returns the slot number.
    pub fn slot(&self) -> u64 {
        self.template.slot()
    }

    /// Returns whether this duty should be executed now.
    pub fn is_ready(&self) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        now >= self.target_timestamp()
    }

    /// Returns how long to wait before executing this duty.
    pub fn wait_duration(&self) -> Option<std::time::Duration> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        if now >= self.target_timestamp() {
            None
        } else {
            Some(std::time::Duration::from_millis(
                self.target_timestamp() - now,
            ))
        }
    }

    pub(crate) fn new(template: FullBlockTemplate) -> Self {
        Self { template }
    }
}

impl CheckpointSigningDuty {
    /// Returns the checkpoint epoch.
    pub fn epoch(&self) -> u32 {
        self.checkpoint.new_tip().epoch
    }

    /// Returns the checkpoint hash.
    pub fn hash(&self) -> [u8; 32] {
        let bytes = self.checkpoint.as_ssz_bytes();
        hash::raw(&bytes).into()
    }
}

#[cfg(test)]
mod tests {
    use strata_checkpoint_types_ssz::{CheckpointSidecar, CheckpointTip};
    use strata_ol_chain_types_new::{BlockFlags, OLBlockBody, OLBlockHeader, OLTxSegment};
    use strata_primitives::OLBlockCommitment;

    use super::*;

    fn create_checkpoint_payload(epoch: u32) -> CheckpointPayload {
        let tip = CheckpointTip {
            epoch,
            l1_height: 200,
            l2_commitment: OLBlockCommitment::new(1, Buf32::zero().into()),
        };
        let sidecar = CheckpointSidecar {
            ol_state_diff: vec![2; 100].into(),
            ol_logs: vec![].into(),
        };
        CheckpointPayload {
            new_tip: tip,
            sidecar,
            proof: vec![0].into(),
        }
    }

    fn create_test_template(timestamp: u64, slot: u64, epoch: u32) -> FullBlockTemplate {
        let header = OLBlockHeader {
            parent_blkid: OLBlockId::from(Buf32([1u8; 32])),
            timestamp,
            slot,
            epoch,
            flags: BlockFlags::from(0),
            body_root: [0u8; 32].into(),
            state_root: [0u8; 32].into(),
            logs_root: [0u8; 32].into(),
        };

        let body = OLBlockBody {
            tx_segment: Some(OLTxSegment { txs: vec![].into() }).into(),
            l1_update: None.into(),
        };

        FullBlockTemplate::new(header, body)
    }

    #[test]
    fn test_block_signing_duty_is_ready() {
        let now_millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        // Create template with timestamp in the past
        let past_template = create_test_template(now_millis - 1000, 1, 0);
        let past_duty = BlockSigningDuty::new(past_template);
        assert!(past_duty.is_ready(), "Past timestamp should be ready");

        // Create template with timestamp in the future
        let future_template = create_test_template(now_millis + 10000, 2, 0);
        let future_duty = BlockSigningDuty::new(future_template);
        assert!(
            !future_duty.is_ready(),
            "Future timestamp should not be ready"
        );

        // Create template with timestamp exactly now
        let now_template = create_test_template(now_millis, 3, 0);
        let now_duty = BlockSigningDuty::new(now_template);
        assert!(now_duty.is_ready(), "Current timestamp should be ready");
    }

    #[test]
    fn test_block_signing_duty_wait_duration() {
        let now_millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        // Past timestamp should return None
        let past_template = create_test_template(now_millis - 1000, 1, 0);
        let past_duty = BlockSigningDuty::new(past_template);
        assert_eq!(
            past_duty.wait_duration(),
            None,
            "Past timestamp should have no wait"
        );

        // Future timestamp should return a duration
        let future_millis = 5000u64;
        let future_template = create_test_template(now_millis + future_millis, 2, 0);
        let future_duty = BlockSigningDuty::new(future_template);

        if let Some(duration) = future_duty.wait_duration() {
            let wait_millis = duration.as_millis() as u64;
            // Allow some tolerance for timing
            assert!(
                wait_millis >= future_millis - 100 && wait_millis <= future_millis + 100,
                "Wait duration should be approximately {} ms, got {} ms",
                future_millis,
                wait_millis
            );
        } else {
            panic!("Future timestamp should have a wait duration");
        }
    }

    #[test]
    fn test_block_signing_duty_accessors() {
        let template = create_test_template(1000, 42, 5);
        let duty = BlockSigningDuty::new(template.clone());

        assert_eq!(duty.template_id(), template.template_id());
        assert_eq!(duty.slot(), 42);
        assert_eq!(duty.target_timestamp(), 1000);
    }

    #[test]
    fn test_duty_generate_id_for_block() {
        let template = create_test_template(1000, 1, 0);
        let duty = Duty::SignBlock(BlockSigningDuty::new(template.clone()));

        let id = duty.generate_id();
        let expected_id: Buf32 = template.template_id().into();
        assert_eq!(id, expected_id);
    }

    #[test]
    fn test_duty_generate_id_for_checkpoint() {
        let checkpoint = create_checkpoint_payload(0);

        let duty = Duty::SignCheckpoint(CheckpointSigningDuty {
            checkpoint: checkpoint.clone(),
        });

        let id = duty.generate_id();
        let encoded = checkpoint.as_ssz_bytes();
        let expected_id = hash::raw(&encoded);
        assert_eq!(id, expected_id);
    }

    #[test]
    fn test_duty_expiry() {
        // Block duty expires on next block
        let template = create_test_template(1000, 1, 0);
        let block_duty = Duty::SignBlock(BlockSigningDuty::new(template));
        assert!(matches!(block_duty.expiry(), Expiry::NextBlock));

        // Checkpoint duty expires when checkpoint is finalized
        let ep = 5;
        let checkpoint = create_checkpoint_payload(ep);

        let checkpoint_duty = Duty::SignCheckpoint(CheckpointSigningDuty { checkpoint });
        assert!(matches!(
            checkpoint_duty.expiry(),
            Expiry::CheckpointFinalized(epoch) if epoch == ep
        ));
    }

    #[test]
    fn test_checkpoint_signing_duty_accessors() {
        let ep = 7;
        let checkpoint = create_checkpoint_payload(ep);

        let duty = CheckpointSigningDuty {
            checkpoint: checkpoint.clone(),
        };

        assert_eq!(duty.epoch(), ep);

        let hash = duty.hash();
        let expected_hash: [u8; 32] = hash::raw(&checkpoint.as_ssz_bytes()).into();
        assert_eq!(hash, expected_hash);
    }
}
