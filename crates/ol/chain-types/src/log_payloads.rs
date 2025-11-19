//! Log payload types for orchestration layer logs.

use strata_acct_types::VarVec;
use strata_codec::{Codec, CodecError, Decoder, Encoder};

/// Payload for a simple withdrawal intent log.
///
/// This log is emitted when a withdrawal intent is created through the bridge.
/// It contains the withdrawal amount and destination information.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimpleWithdrawalIntentLogData {
    /// Amount to withdraw (in satoshis).
    pub amt: u64,
    /// Destination address or script (BOSD - Bitcoin Output Script Descriptor).
    pub dest: VarVec<u8>,
}

impl SimpleWithdrawalIntentLogData {
    /// Create a new simple withdrawal intent log data instance.
    pub fn new(amt: u64, dest: Vec<u8>) -> Option<Self> {
        let dest = VarVec::from_vec(dest)?;
        Some(Self { amt, dest })
    }

    /// Get the withdrawal amount.
    pub fn amt(&self) -> u64 {
        self.amt
    }

    /// Get the destination as bytes.
    pub fn dest(&self) -> &[u8] {
        self.dest.as_ref()
    }
}

// Codec implementation for SimpleWithdrawalIntentLogData
impl Codec for SimpleWithdrawalIntentLogData {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.amt.encode(enc)?;
        self.dest.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let amt = u64::decode(dec)?;
        let dest = VarVec::<u8>::decode(dec)?;
        Ok(Self { amt, dest })
    }
}