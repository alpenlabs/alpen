//! RPC types for sequencer duties and block templates.

use serde::{Deserialize, Serialize};
use strata_identifiers::{Epoch, OLBlockCommitment, OLBlockId};
use strata_primitives::HexBytes;

/// Sequencer duty for OL.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RpcOLDuty {
    /// Sign an OL block.
    SignBlock(RpcOLBlockSigningDuty),

    /// Commit a checkpoint batch.
    CommitBatch(RpcOLCheckpointDuty),
}

/// Block signing duty for OL blocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcOLBlockSigningDuty {
    /// Slot to sign for.
    slot: u64,

    /// Parent to build on.
    parent: OLBlockId,

    /// Target timestamp for block.
    target_ts: u64,
}

impl RpcOLBlockSigningDuty {
    /// Create new instance.
    pub fn new(slot: u64, parent: OLBlockId, target_ts: u64) -> Self {
        Self {
            slot,
            parent,
            target_ts,
        }
    }

    /// Returns target slot.
    pub fn target_slot(&self) -> u64 {
        self.slot
    }

    /// Returns parent block id.
    pub fn parent(&self) -> OLBlockId {
        self.parent
    }

    /// Returns target timestamp.
    pub fn target_ts(&self) -> u64 {
        self.target_ts
    }
}

/// Checkpoint duty for OL checkpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcOLCheckpointDuty {
    /// Checkpoint epoch.
    epoch: Epoch,

    /// SSZ-encoded [`CheckpointPayload`].
    checkpoint_payload: HexBytes,
}

impl RpcOLCheckpointDuty {
    /// Create new instance from epoch and payload bytes.
    pub fn new(epoch: Epoch, checkpoint_payload: HexBytes) -> Self {
        Self {
            epoch,
            checkpoint_payload,
        }
    }

    /// Returns checkpoint epoch.
    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    /// Returns SSZ-encoded checkpoint payload bytes.
    pub fn checkpoint_payload(&self) -> &HexBytes {
        &self.checkpoint_payload
    }
}

/// OL block template for signing (SSZ-encoded header).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcOLBlockTemplate {
    /// Template id (block id of header).
    template_id: OLBlockId,

    /// SSZ-encoded `OLBlockHeader` bytes.
    header: HexBytes,
}

impl RpcOLBlockTemplate {
    /// Create a new template.
    pub fn new(template_id: OLBlockId, header: HexBytes) -> Self {
        Self {
            template_id,
            header,
        }
    }

    /// Returns template id.
    pub fn template_id(&self) -> OLBlockId {
        self.template_id
    }

    /// Returns SSZ-encoded header bytes.
    pub fn header(&self) -> &HexBytes {
        &self.header
    }
}

/// Configuration provided by sequencer for block generation.
#[derive(Debug, Clone, Serialize, Deserialize, Hash, Eq, PartialEq)]
pub struct RpcBlockGenerationConfig {
    parent_block_commitment: OLBlockCommitment,
    #[serde(skip_serializing_if = "Option::is_none")]
    ts: Option<u64>,
}

impl RpcBlockGenerationConfig {
    /// Create new instance with provided parent block commitment.
    pub fn new(parent_block_commitment: OLBlockCommitment) -> Self {
        Self {
            parent_block_commitment,
            ts: None,
        }
    }

    /// Update with provided block timestamp.
    pub fn with_ts(mut self, ts: u64) -> Self {
        self.ts = Some(ts);
        self
    }

    /// Return parent block commitment.
    pub fn parent_block_commitment(&self) -> OLBlockCommitment {
        self.parent_block_commitment
    }

    /// Return block timestamp.
    pub fn ts(&self) -> Option<u64> {
        self.ts
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use strata_identifiers::{Buf32, OLBlockId};
    use strata_primitives::HexBytes;

    use super::*;

    proptest! {
        #[test]
        fn rpc_block_generation_config_roundtrip(
            slot in any::<u64>(),
            blkid_bytes in any::<[u8; 32]>(),
            ts in prop::option::of(any::<u64>()),
        ) {
            let blkid = OLBlockId::from(Buf32::from(blkid_bytes));
            let commitment = OLBlockCommitment::new(slot, blkid);
            let cfg = RpcBlockGenerationConfig::new(commitment);
            let cfg = if let Some(ts) = ts { cfg.with_ts(ts) } else { cfg };

            let json = serde_json::to_string(&cfg).expect("serialize");
            let decoded: RpcBlockGenerationConfig = serde_json::from_str(&json).expect("deserialize");
            prop_assert_eq!(cfg, decoded);
        }

        #[test]
        fn rpc_duty_roundtrip(
            epoch in any::<u32>(),
            payload in prop::collection::vec(any::<u8>(), 0..64),
        ) {
            let duty = RpcOLDuty::CommitBatch(RpcOLCheckpointDuty::new(
                epoch,
                HexBytes(payload),
            ));

            let json = serde_json::to_string(&duty).expect("serialize");
            let decoded: RpcOLDuty = serde_json::from_str(&json).expect("deserialize");
            prop_assert_eq!(json, serde_json::to_string(&decoded).expect("serialize"));
        }
    }
}
