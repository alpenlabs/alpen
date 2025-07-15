use borsh::{BorshDeserialize, BorshSerialize};
use moho_types::InnerVerificationKey;
use strata_msg_fmt::TypeId;

use crate::logs::{AsmLog, constants::ASM_STF_UPDATE_LOG_TYPE};

/// Details for an execution environment verification key update.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct AsmStfUpdate {
    /// New execution environment state transition function verification key.
    pub new_vk: InnerVerificationKey,
}

impl AsmLog for AsmStfUpdate {
    const TY: TypeId = ASM_STF_UPDATE_LOG_TYPE;
}
