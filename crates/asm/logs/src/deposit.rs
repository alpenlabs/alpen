use strata_asm_common::AsmLog;
use strata_codec::{Codec, VarVec};
use strata_msg_fmt::TypeId;

use crate::constants::DEPOSIT_LOG_TYPE_ID;

/// Details for a deposit operation.
#[derive(Debug, Clone, Codec)]
pub struct DepositLog {
    /// Destination
    pub destination: VarVec<u8>,
    /// Amount in satoshis.
    pub amount: u64,
}

impl DepositLog {
    /// Create a new DepositLog instance.
    pub fn new(destination: VarVec<u8>, amount: u64) -> Self {
        Self {
            destination,
            amount,
        }
    }
}

impl AsmLog for DepositLog {
    const TY: TypeId = DEPOSIT_LOG_TYPE_ID;
}
