use borsh::{BorshDeserialize, BorshSerialize};
use moho_types::InnerVerificationKey;
use strata_asm_common::{AsmLog, ASM_STF_UPDATE_LOG_TYPE};
use strata_msg_fmt::TypeId;

/// Details for an execution environment verification key update.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct AsmStfUpdate {
    /// New execution environment state transition function verification key.
    pub new_vk: InnerVerificationKey,
}

impl AsmStfUpdate {
    /// Create a new AsmStfUpdate instance.
    pub fn new(new_vk: InnerVerificationKey) -> Self {
        Self { new_vk }
    }
}

impl AsmLog for AsmStfUpdate {
    const TY: TypeId = ASM_STF_UPDATE_LOG_TYPE;
}
