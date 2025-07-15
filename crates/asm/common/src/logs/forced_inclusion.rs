use borsh::{BorshDeserialize, BorshSerialize};
use strata_msg_fmt::TypeId;

use crate::logs::{AsmLog, constants::DEPOSIT_LOG_TYPE_ID};

/// Details for a forced inclusion operation.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct ForcedInclusionData {
    /// Identifier of the target execution environment.
    pub ee_id: u64,
    /// Raw payload data for inclusion.
    pub payload: Vec<u8>,
}

impl AsmLog for ForcedInclusionData {
    const TY: TypeId = DEPOSIT_LOG_TYPE_ID;
}
