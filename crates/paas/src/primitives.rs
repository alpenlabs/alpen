//! Integration with strata-primitives types
//!
//! This module provides ProgramId implementations for strata-primitives types,
//! avoiding orphan rule violations.

use serde::{Deserialize, Serialize};
use strata_primitives::proof::ProofContext;

use crate::registry::ProgramType;
use crate::zkvm::ProgramId;

/// Routing key for ProofContext - used for dynamic dispatch in PaaS registry
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProofContextVariant {
    EvmEeStf,
    ClStf,
    Checkpoint,
}

/// Implement ProgramId for ProofContext to allow using it as a program identifier in PaaS (legacy API)
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

/// Implement ProgramType for ProofContext to enable registry-based PaaS API
impl ProgramType for ProofContext {
    type RoutingKey = ProofContextVariant;

    fn routing_key(&self) -> Self::RoutingKey {
        match self {
            ProofContext::EvmEeStf(..) => ProofContextVariant::EvmEeStf,
            ProofContext::ClStf(..) => ProofContextVariant::ClStf,
            ProofContext::Checkpoint(_) => ProofContextVariant::Checkpoint,
        }
    }
}
