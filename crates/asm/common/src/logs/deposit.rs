use borsh::{BorshDeserialize, BorshSerialize};

use crate::logs::AsmLog;

pub const DEPOSIT_LOG_TYPE_ID: u16 = 1;

/// Details for a deposit operation.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct DepositLog {
    /// Identifier of the target execution environment.
    pub ee_id: u64,
    /// Amount in satoshis.
    pub amount: u64,
    /// Serialized address for the operation.
    pub addr: Vec<u8>,
}

impl AsmLog for DepositLog {
    fn ty() -> strata_msg_fmt::TypeId {
        1
    }

    fn as_dyn_any(&self) -> &dyn std::any::Any {
        self
    }
}
