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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepositIntentLogData {
    /// Serial number of the destination account.
    pub dest_acct_serial: AccountSerial,
    /// Subject ID within the destination account.
    pub dest_subject: SubjectId,
    /// Amount to deposit (in satoshis).
    pub amt: u64,
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
}

// Codec implementation for DepositIntentLogData
impl Codec for DepositIntentLogData {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.dest_acct_serial.encode(enc)?;
        self.dest_subject.encode(enc)?;
        self.amt.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let dest_acct_serial = AccountSerial::decode(dec)?;
        let dest_subject = SubjectId::decode(dec)?;
        let amt = u64::decode(dec)?;
        Ok(Self {
            dest_acct_serial,
            dest_subject,
            amt,
        })
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
    pub epoch: EpochCommitment,
}

impl CheckpointAckLogData {
    /// Create a new checkpoint acknowledgment log data instance.
    pub fn new(epoch: EpochCommitment) -> Self {
        Self { epoch }
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