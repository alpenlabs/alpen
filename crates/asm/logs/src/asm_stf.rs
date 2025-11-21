use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::AsmLog;
use strata_codec_derive::Codec;
use strata_codec_utils::CodecBorsh;
use strata_msg_fmt::TypeId;
use strata_predicate::PredicateKey;

use crate::constants::ASM_STF_UPDATE_LOG_TYPE;

/// Details for an execution environment verification key update.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize, Codec)]
pub struct AsmStfUpdate {
    /// New execution environment state transition function verification key.
    new_predicate: CodecBorsh<PredicateKey>,
}

impl AsmStfUpdate {
    /// Create a new AsmStfUpdate instance.
    pub fn new(new_predicate: PredicateKey) -> Self {
        Self {
            new_predicate: CodecBorsh::new(new_predicate),
        }
    }

    pub fn new_predicate(&self) -> &PredicateKey {
        self.new_predicate.inner()
    }

    pub fn into_new_predicate(self) -> PredicateKey {
        self.new_predicate.into_inner()
    }
}

impl AsmLog for AsmStfUpdate {
    const TY: TypeId = ASM_STF_UPDATE_LOG_TYPE;
}
