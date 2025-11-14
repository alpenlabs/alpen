use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::AsmLog;
use strata_msg_fmt::TypeId;
use strata_predicate::PredicateKey;

use crate::constants::LogTypeId;

/// Details for an execution environment verification key update.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct AsmStfUpdate {
    /// New execution environment state transition function verification key.
    pub new_predicate: PredicateKey,
}

impl AsmStfUpdate {
    /// Create a new AsmStfUpdate instance.
    pub fn new(new_predicate: PredicateKey) -> Self {
        Self { new_predicate }
    }
}

impl AsmLog for AsmStfUpdate {
    const TY: TypeId = LogTypeId::AsmStfUpdate as u16;
}
