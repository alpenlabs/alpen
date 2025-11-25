use strata_asm_common::AsmLog;
use strata_codec::{Codec, VarVec};
use strata_msg_fmt::TypeId;

use crate::constants::DEPOSIT_LOG_TYPE_ID;

/// Details for a deposit operation.
#[derive(Debug, Clone, Codec)]
pub struct DepositLog {
    /// Identifier of the target execution environment.
    pub ee_id: u64,
    /// Amount in satoshis.
    pub amount: u64,
    /// Serialized address for the operation.
    pub addr: VarVec<u8>,
}

impl DepositLog {
    /// Create a new DepositLog instance.
    pub fn new(ee_id: u64, amount: u64, addr: Vec<u8>) -> Self {
        Self {
            ee_id,
            amount,
            addr: VarVec::from_vec(addr).expect("address too large for VarVec"),
        }
    }
}

impl AsmLog for DepositLog {
    const TY: TypeId = DEPOSIT_LOG_TYPE_ID;
}
