use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::AsmLog;
use strata_msg_fmt::TypeId;

use crate::constants::DEPOSIT_LOG_TYPE_ID;

/// Details for a deposit operation.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct DepositLog {
    /// Identifier of the target execution environment.
    pub ee_id: u64, // TODO: rename to acc_id/acc_serial. And type to u32 ?
    /// Amount in satoshis.
    pub amount: u64,
    /// Serialized address for the operation.
    pub addr: Vec<u8>,
}

impl DepositLog {
    /// Create a new DepositLog instance.
    pub fn new(ee_id: u64, amount: u64, addr: Vec<u8>) -> Self {
        Self {
            ee_id,
            amount,
            addr,
        }
    }
}

impl AsmLog for DepositLog {
    const TY: TypeId = DEPOSIT_LOG_TYPE_ID;
}
