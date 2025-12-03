//! ProofTask type for Prover Service integration
//!
//! This module defines the ProofTask newtype wrapper around ProofContext
//! and implements the ProgramType trait for registry-based dispatch.

use serde::{Deserialize, Serialize};
use strata_paas::ProgramType;
use strata_primitives::proof::ProofContext;

/// Routing key for ProofContext - used for dynamic dispatch in Prover Service registry
///
/// This enum represents the different variants of ProofContext without carrying
/// the actual data, allowing the registry to route requests to the correct handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProofContextVariant {
    EvmEeStf,
    ClStf,
    Checkpoint,
}

/// Newtype wrapper for ProofContext that allows us to implement foreign traits
///
/// This wrapper works around Rust's orphan rule, which prevents implementing
/// foreign traits on foreign types. By wrapping ProofContext, we can implement
/// Prover Service traits in this crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProofTask(pub ProofContext);

impl From<ProofContext> for ProofTask {
    fn from(ctx: ProofContext) -> Self {
        ProofTask(ctx)
    }
}

impl From<ProofTask> for ProofContext {
    fn from(task: ProofTask) -> Self {
        task.0
    }
}

impl std::ops::Deref for ProofTask {
    type Target = ProofContext;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Implement ProgramType for ProofTask to enable registry-based Prover Service API
impl ProgramType for ProofTask {
    type RoutingKey = ProofContextVariant;

    fn routing_key(&self) -> Self::RoutingKey {
        match self.0 {
            ProofContext::EvmEeStf(..) => ProofContextVariant::EvmEeStf,
            ProofContext::ClStf(..) => ProofContextVariant::ClStf,
            ProofContext::Checkpoint(_) => ProofContextVariant::Checkpoint,
        }
    }
}
