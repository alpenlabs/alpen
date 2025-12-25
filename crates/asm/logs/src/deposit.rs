use strata_asm_common::AsmLog;
use strata_codec::Codec;
use strata_identifiers::{AccountSerial, SubjectId};
use strata_msg_fmt::TypeId;

use crate::constants::DEPOSIT_LOG_TYPE_ID;

/// Details for a deposit operation.
#[derive(Debug, Clone, Codec)]
pub struct DepositLog {
    /// Identifier of the target execution environment.
    pub ee_id: AccountSerial,
    /// Amount in satoshis.
    pub amount: u64,
    /// Serialized address for the operation.
    pub addr: SubjectId,
}

impl DepositLog {
    /// Create a new DepositLog instance.
    pub fn new(ee_id: AccountSerial, amount: u64, addr: SubjectId) -> Self {
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
