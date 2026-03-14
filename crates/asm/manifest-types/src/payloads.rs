//! ASM log payload types.
//!
//! This module contains the actual payload formats for ASM logs that are produced
//! by the ASM and processed by the orchestration layer state transition function.

use strata_codec::{Codec, CodecError, Decoder, Encoder};
use strata_identifiers::EpochCommitment;
use strata_msg_fmt::TypeId;

use crate::AsmLog;

// Define type IDs for these log types
pub const CHECKPOINT_ACK_ASM_LOG_TYPE_ID: TypeId = 101; // TODO: Confirm the actual type ID

/// Payload for a checkpoint acknowledgment ASM log.
///
/// This represents the orchestration layer acknowledging that a checkpoint
/// has been recorded on L1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointAckLogData {
    /// The epoch commitment that was acknowledged.
    epoch: EpochCommitment,
}

impl CheckpointAckLogData {
    /// Create a new checkpoint acknowledgment log data instance.
    pub fn new(epoch: EpochCommitment) -> Self {
        Self { epoch }
    }

    pub fn epoch(&self) -> EpochCommitment {
        self.epoch
    }
}

// Codec implementation for CheckpointAckLogData
impl Codec for CheckpointAckLogData {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.epoch.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let epoch = EpochCommitment::decode(dec)?;
        Ok(Self { epoch })
    }
}

impl AsmLog for CheckpointAckLogData {
    const TY: TypeId = CHECKPOINT_ACK_ASM_LOG_TYPE_ID;
}

#[cfg(test)]
mod tests {
    use strata_codec::{decode_buf_exact, encode_to_vec};
    use strata_identifiers::{Buf32, OLBlockId};

    use super::*;

    #[test]
    fn test_checkpoint_ack_log_data_codec() {
        // Create test data
        let epoch_commitment = EpochCommitment::new(
            100, // RawEpoch is just u32
            999, // last_slot is u64
            OLBlockId::from(Buf32::from([2u8; 32])),
        );
        let log_data = CheckpointAckLogData {
            epoch: epoch_commitment,
        };

        // Encode
        let encoded = encode_to_vec(&log_data).unwrap();

        // Decode
        let decoded: CheckpointAckLogData = decode_buf_exact(&encoded).unwrap();

        // Verify round-trip
        assert_eq!(decoded.epoch.epoch(), log_data.epoch.epoch());
        assert_eq!(decoded.epoch.last_slot(), log_data.epoch.last_slot());
        assert_eq!(decoded.epoch.last_blkid(), log_data.epoch.last_blkid());
    }
}
