//! Genesis configuration management for ASM subprotocols.
//!
//! This module provides a registry for managing genesis state of subprotocol
//! that are used to initialize subprotocol states during genesis phase processing
//! or when new subprotocols are added.

use std::collections::BTreeMap;

use borsh::{BorshDeserialize, BorshSerialize};
use strata_l1_txfmt::SubprotocolId;

use crate::{AsmError, Subprotocol};

/// Registry for managing genesis state for all subprotocols.
///
/// This registry stores serialized genesis state that are used
/// when initializing subprotocol states. The state are keyed
/// by subprotocol ID and stored in serialized form to avoid type dependencies.
#[derive(Debug, Clone, Default, BorshSerialize, BorshDeserialize)]
pub struct GenesisConfigRegistry {
    /// Map of subprotocol ID to serialized genesis state
    configs: BTreeMap<SubprotocolId, Vec<u8>>,
}

// [PLACE_HOLDER] => TODO: Do we have a better alternative approach that simplifies and improves
// state initialization? the current approach add genesis management complexity to ASM upper layer
impl GenesisConfigRegistry {
    pub fn new() -> Self {
        Self {
            configs: BTreeMap::new(),
        }
    }

    /// Registers a genesis configuration for a subprotocol.
    ///
    /// # Arguments
    /// * `config` - The genesis configuration to register
    pub fn register<S: Subprotocol>(&mut self, config: &S::GenesisConfig) -> Result<(), AsmError> {
        let id = S::ID;
        let serialized = borsh::to_vec(config).map_err(|e| AsmError::Serialization(id, e))?;
        self.configs.insert(id, serialized);
        Ok(())
    }

    /// Retrieves and deserializes a genesis configuration for a subprotocol.
    ///
    /// # Arguments
    /// * `id` - The subprotocol ID
    ///
    /// # Returns
    /// The deserialized genesis configuration or None if not found
    pub fn get<T: BorshDeserialize>(&self, id: SubprotocolId) -> Option<T> {
        self.configs
            .get(&id)
            .and_then(|data| borsh::from_slice(data).ok())
    }

    /// Retrieves the raw serialized genesis configuration data for a subprotocol.
    ///
    /// # Arguments
    /// * `id` - The subprotocol ID
    ///
    /// # Returns
    /// A reference to the serialized configuration data or None if not found
    pub fn get_raw(&self, id: SubprotocolId) -> Option<&[u8]> {
        self.configs.get(&id).map(|v| v.as_slice())
    }

    /// Checks if a genesis configuration exists for a subprotocol.
    pub fn contains(&self, id: SubprotocolId) -> bool {
        self.configs.contains_key(&id)
    }

    /// Returns the number of registered genesis configurations.
    pub fn len(&self) -> usize {
        self.configs.len()
    }

    /// Returns true if the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.configs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AnchorState, MsgRelayer, TxInputRef};

    #[derive(Debug, PartialEq, BorshSerialize, BorshDeserialize)]
    struct TestConfig {
        value: u32,
    }

    #[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
    struct TestState {
        value: u32,
    }

    struct TestSubprotocol;

    impl Subprotocol for TestSubprotocol {
        const ID: SubprotocolId = 1;
        type State = TestState;
        type Msg = ();
        type AuxInput = ();
        type GenesisConfig = TestConfig;

        fn init(genesis_config: Self::GenesisConfig) -> Result<Self::State, AsmError> {
            Ok(TestState {
                value: genesis_config.value,
            })
        }

        fn process_txs(
            _state: &mut Self::State,
            _txs: &[TxInputRef<'_>],
            _anchor_pre: &AnchorState,
            _aux_inputs: &[Self::AuxInput],
            _relayer: &mut impl MsgRelayer,
        ) {
            // No-op for test
        }

        fn process_msgs(_state: &mut Self::State, _msgs: &[Self::Msg]) {
            // No-op for test
        }
    }

    #[test]
    fn test_genesis_registry() {
        let mut registry = GenesisConfigRegistry::new();
        let config = TestConfig { value: 42 };

        // Register config
        registry.register::<TestSubprotocol>(&config).unwrap();
        assert!(registry.contains(1));
        assert_eq!(registry.len(), 1);

        // Retrieve config
        let retrieved: TestConfig = registry.get(1).unwrap();
        assert_eq!(retrieved, config);

        // Non-existent config
        let missing: Option<TestConfig> = registry.get(2);
        assert!(missing.is_none());
    }
}
