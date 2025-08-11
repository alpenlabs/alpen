use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::AsmLog;
use strata_msg_fmt::TypeId;
use zkaleido::VerifyingKey;

use crate::constants::ASM_STF_UPDATE_LOG_TYPE;

/// Details for an execution environment verification key update.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct AsmStfUpdate {
    /// New execution environment state transition function verification key.
    pub new_vk: VerifyingKey,
}

impl AsmStfUpdate {
    /// Create a new AsmStfUpdate instance.
    pub fn new(new_vk: VerifyingKey) -> Self {
        Self { new_vk }
    }
}

impl AsmLog for AsmStfUpdate {
    const TY: TypeId = ASM_STF_UPDATE_LOG_TYPE;
}
