use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::AsmLog;
use strata_msg_fmt::TypeId;

use crate::constants::FORCED_INCLUSION_LOG_TYPE_ID;

/// Details for a forced inclusion operation.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct ForcedInclusionData {
    /// Identifier of the target execution environment.
    pub ee_id: u64,
    /// Raw payload data for inclusion.
    pub payload: Vec<u8>,
}

impl ForcedInclusionData {
    /// Create a new ForcedInclusionData instance.
    pub fn new(ee_id: u64, payload: Vec<u8>) -> Self {
        Self { ee_id, payload }
    }
}

impl AsmLog for ForcedInclusionData {
    const TY: TypeId = FORCED_INCLUSION_LOG_TYPE_ID;
}
