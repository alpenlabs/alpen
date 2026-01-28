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
