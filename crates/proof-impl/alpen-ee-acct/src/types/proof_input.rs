//! Alpen EE proof input type

use rsp_primitives::genesis::Genesis;

use super::{EeAccountInit, RuntimeUpdateInput};

/// Private inputs to the proof for an Alpen EVM EE (runtime) account update.
///
/// This struct contains all the data needed by the guest zkVM to verify and apply
/// an EE account update operation for Alpen's EVM execution environment.
///
/// The fields are logically grouped into:
/// - Account initialization data
/// - Runtime update inputs
/// - EVM executor configuration
#[derive(Debug, Clone)]
pub struct AlpenEeProofInput {
    /// Fields that initialize the EE account
    account_init: EeAccountInit,

    /// Fields we plug into the EE runtime to process the update
    runtime_input: RuntimeUpdateInput,

    /// Genesis data for constructing ChainSpec (serde-serialized)
    genesis: Genesis,
}

impl AlpenEeProofInput {
    /// Create a new AlpenEeProofInput
    pub fn new(
        account_init: EeAccountInit,
        runtime_input: RuntimeUpdateInput,
        genesis: Genesis,
    ) -> Self {
        Self {
            account_init,
            runtime_input,
            genesis,
        }
    }

    /// Get reference to account initialization data
    pub fn account_init(&self) -> &EeAccountInit {
        &self.account_init
    }

    /// Get reference to runtime update input
    pub fn runtime_input(&self) -> &RuntimeUpdateInput {
        &self.runtime_input
    }

    /// Get reference to genesis data
    pub fn genesis(&self) -> &Genesis {
        &self.genesis
    }
}
