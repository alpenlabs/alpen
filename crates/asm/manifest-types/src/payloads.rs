//! ASM log payload types.
//!
//! This module contains the actual payload formats for ASM logs that are produced
//! by the ASM and processed by the orchestration layer state transition function.

use strata_codec::{Codec, CodecError, Decoder, Encoder};
use strata_identifiers::{AccountSerial, EpochCommitment, SubjectId};
use strata_msg_fmt::TypeId;

use crate::AsmLog;

// Define type IDs for these log types
pub const DEPOSIT_INTENT_ASM_LOG_TYPE_ID: TypeId = 100; // TODO: Confirm the actual type ID
pub const CHECKPOINT_ACK_ASM_LOG_TYPE_ID: TypeId = 101; // TODO: Confirm the actual type ID

/// Payload for a deposit intent ASM log.
///
/// This represents a deposit operation targeting a specific account and subject
/// in the orchestration layer.
#[derive(Debug, Clone, PartialEq, Eq, Codec)]
pub struct DepositIntentLogData {
    /// Serial number of the destination account.
    dest_acct_serial: AccountSerial,

    /// Subject ID within the destination account.
    dest_subject: SubjectId,

    /// Amount to deposit (in satoshis).
    amt: u64,
}

impl DepositIntentLogData {
    /// Create a new deposit intent log data instance.
    pub fn new(dest_acct_serial: AccountSerial, dest_subject: SubjectId, amt: u64) -> Self {
        Self {
            dest_acct_serial,
            dest_subject,
            amt,
        }
    }

    pub fn dest_acct_serial(&self) -> AccountSerial {
        self.dest_acct_serial
    }

    pub fn dest_subject(&self) -> SubjectId {
        self.dest_subject
    }

    pub fn amt(&self) -> u64 {
        self.amt
    }
}

impl AsmLog for DepositIntentLogData {
    const TY: TypeId = DEPOSIT_INTENT_ASM_LOG_TYPE_ID;
}

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
    use strata_identifiers::{AccountSerial, Buf32, OLBlockId, SubjectId};

    use super::*;

    #[test]
    fn test_deposit_intent_log_data_codec() {
        // Create test data
        let log_data = DepositIntentLogData {
            dest_acct_serial: AccountSerial::from(42),
            dest_subject: SubjectId::from([1u8; 32]),
            amt: 100_000_000,
        };

        // Encode
        let encoded = encode_to_vec(&log_data).unwrap();

        // Decode
        let decoded: DepositIntentLogData = decode_buf_exact(&encoded).unwrap();

        // Verify round-trip
        assert_eq!(decoded.dest_acct_serial, log_data.dest_acct_serial);
        assert_eq!(decoded.dest_subject, log_data.dest_subject);
        assert_eq!(decoded.amt, log_data.amt);
    }

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

    #[test]
    fn test_deposit_intent_zero_amount() {
        // Test with zero amount
        let log_data = DepositIntentLogData {
            dest_acct_serial: AccountSerial::from(0),
            dest_subject: SubjectId::from([0u8; 32]),
            amt: 0,
        };

        let encoded = encode_to_vec(&log_data).unwrap();
        let decoded: DepositIntentLogData = decode_buf_exact(&encoded).unwrap();

        assert_eq!(decoded.amt, 0);
        assert_eq!(decoded.dest_acct_serial, AccountSerial::from(0));
    }

    #[test]
    fn test_deposit_intent_max_values() {
        // Test with max values
        let log_data = DepositIntentLogData {
            dest_acct_serial: AccountSerial::from(u32::MAX),
            dest_subject: SubjectId::from([255u8; 32]),
            amt: u64::MAX,
        };

        let encoded = encode_to_vec(&log_data).unwrap();
        let decoded: DepositIntentLogData = decode_buf_exact(&encoded).unwrap();

        assert_eq!(decoded.dest_acct_serial, AccountSerial::from(u32::MAX));
        assert_eq!(decoded.dest_subject, SubjectId::from([255u8; 32]));
        assert_eq!(decoded.amt, u64::MAX);
    }
}
