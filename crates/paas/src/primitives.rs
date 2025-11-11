//! Integration with strata-primitives types
//!
//! This module provides ProgramId implementations for strata-primitives types,
//! avoiding orphan rule violations.

use strata_primitives::proof::ProofContext;

use crate::zkvm::ProgramId;

/// Implement ProgramId for ProofContext to allow using it as a program identifier in PaaS
impl ProgramId for ProofContext {
    fn name(&self) -> String {
        match self {
            ProofContext::Checkpoint(idx) => format!("checkpoint_{}", idx),
            ProofContext::ClStf(start, end) => {
                format!("cl_stf_{}_{}", start.slot(), end.slot())
            }
            ProofContext::EvmEeStf(start, end) => {
                format!("evm_ee_stf_{}_{}", start.slot(), end.slot())
            }
        }
    }
}
