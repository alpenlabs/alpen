//! Genesis configuration builder for the Core subprotocol.
//!
//! This module provides utilities to create and configure the genesis state
//! for the Core subprotocol based on rollup parameters and network configuration.

use strata_asm_common::GenesisConfigRegistry;
use strata_primitives::{
    buf::Buf32,
    l1::L1BlockId,
    l2::{L2BlockCommitment, L2BlockId},
    params::RollupParams,
};
use zkaleido::VerifyingKey;

use crate::{CoreGenesisConfig, CORE_SUBPROTOCOL_ID};

/// Builder for creating Core subprotocol genesis configuration.
#[derive(Debug)]
pub struct CoreGenesisBuilder {
    config: CoreGenesisConfig,
}

impl CoreGenesisBuilder {
    /// Creates a new genesis builder from rollup parameters.
    ///
    /// This extracts relevant configuration from the rollup parameters
    /// to initialize the Core subprotocol state.
    pub fn from_rollup_params(params: &RollupParams) -> Self {
        // Extract the verifying key from rollup params
        let checkpoint_vk = match &params.rollup_vk {
            strata_primitives::proof::RollupVerifyingKey::Risc0VerifyingKey(vk) => {
                VerifyingKey::new(vk.as_ref().to_vec())
            }
            strata_primitives::proof::RollupVerifyingKey::SP1VerifyingKey(vk) => {
                VerifyingKey::new(vk.as_ref().to_vec())
            }
            strata_primitives::proof::RollupVerifyingKey::NativeVerifyingKey(_) => {
                // For native execution, use an empty VK
                VerifyingKey::new(vec![])
            }
        };

        // Create genesis checkpoint from rollup params
        let genesis_checkpoint = strata_primitives::batch::EpochSummary::new(
            0, // Genesis epoch
            L2BlockCommitment::new(0, L2BlockId::from(params.evm_genesis_block_hash)),
            L2BlockCommitment::new(0, L2BlockId::from(params.evm_genesis_block_hash)),
            strata_primitives::l1::L1BlockCommitment::new(
                params.genesis_l1_height,
                L1BlockId::default(), // Should be provided by actual genesis L1 block
            ),
            params.evm_genesis_block_state_root,
        );

        // Extract sequencer key from operator config
        let sequencer_pubkey = match &params.operator_config {
            strata_primitives::params::OperatorConfig::Static(operators) => {
                if let Some(first_op) = operators.first() {
                    // Use the signing key from the first operator
                    *first_op.signing_pk()
                } else {
                    Buf32::zero() // No operators configured
                }
            }
        };

        let config = CoreGenesisConfig {
            checkpoint_vk,
            initial_checkpoint: genesis_checkpoint,
            initial_l1_ref: L1BlockId::default(), // Should be set to actual genesis L1 block
            sequencer_pubkey,
        };

        Self { config }
    }

    /// Sets the initial L1 block reference.
    pub fn with_l1_ref(mut self, l1_ref: L1BlockId) -> Self {
        self.config.initial_l1_ref = l1_ref;
        self
    }

    /// Sets the sequencer public key.
    pub fn with_sequencer_key(mut self, pubkey: Buf32) -> Self {
        self.config.sequencer_pubkey = pubkey;
        self
    }

    /// Sets a custom checkpoint verifying key.
    pub fn with_checkpoint_vk(mut self, vk: VerifyingKey) -> Self {
        self.config.checkpoint_vk = vk;
        self
    }

    /// Builds the genesis configuration.
    pub fn build(self) -> CoreGenesisConfig {
        self.config
    }

    /// Registers the genesis configuration in the provided registry.
    pub fn register_in(self, registry: &mut GenesisConfigRegistry) -> Result<(), strata_asm_common::AsmError> {
        registry.register(CORE_SUBPROTOCOL_ID, &self.config)
    }
}

/// Helper function to create a default genesis registry with Core configuration.
///
/// This is useful for testing and development environments.
pub fn create_default_genesis_registry() -> GenesisConfigRegistry {
    let mut registry = GenesisConfigRegistry::new();
    
    // Register default Core genesis config
    let core_config = CoreGenesisConfig::default();
    registry.register(CORE_SUBPROTOCOL_ID, &core_config)
        .expect("Failed to register core genesis config");
    
    // Note: Bridge genesis config should be registered separately by the bridge module
    
    registry
}

#[cfg(test)]
mod tests {
    use super::*;
    use strata_primitives::{
        block_credential::CredRule,
        params::{OperatorConfig, ProofPublishMode},
    };

    fn create_test_rollup_params() -> RollupParams {
        RollupParams {
            rollup_name: "test-rollup".to_string(),
            block_time: 1000,
            da_tag: "test-da".to_string(),
            checkpoint_tag: "test-ckpt".to_string(),
            cred_rule: CredRule::Unchecked,
            horizon_l1_height: 0,
            genesis_l1_height: 100,
            operator_config: OperatorConfig::Static(vec![]),
            evm_genesis_block_hash: Buf32::from([1u8; 32]),
            evm_genesis_block_state_root: Buf32::from([2u8; 32]),
            l1_reorg_safe_depth: 3,
            target_l2_batch_size: 64,
            address_length: 20,
            deposit_amount: 1_000_000_000,
            rollup_vk: strata_primitives::proof::RollupVerifyingKey::NativeVerifyingKey(Buf32::zero()),
            dispatch_assignment_dur: 64,
            proof_publish_mode: ProofPublishMode::Timeout(1000),
            max_deposits_in_block: 16,
            network: bitcoin::Network::Regtest,
        }
    }

    #[test]
    fn test_genesis_builder_from_rollup_params() {
        let params = create_test_rollup_params();
        let config = CoreGenesisBuilder::from_rollup_params(&params)
            .with_l1_ref(L1BlockId::from(Buf32::from([3u8; 32])))
            .build();

        assert_eq!(config.initial_checkpoint.epoch(), 0);
        assert_eq!(config.initial_l1_ref, L1BlockId::from(Buf32::from([3u8; 32])));
    }

    #[test]
    fn test_create_default_registry() {
        let registry = create_default_genesis_registry();
        assert!(registry.contains(CORE_SUBPROTOCOL_ID));
    }
}