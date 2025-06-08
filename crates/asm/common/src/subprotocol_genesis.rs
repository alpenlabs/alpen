//! Genesis configuration management for ASM subprotocols.
//!
//! This module provides a registry for managing genesis state of subprotocol
//! that are used to initialize subprotocol states during genesis phase processing
//! or when new subprotocols are added.

use std::collections::BTreeMap;

use borsh::{BorshDeserialize, BorshSerialize};

use crate::{AsmError, SubprotocolId};

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

impl GenesisConfigRegistry {
    pub fn new() -> Self {
        Self {
            configs: BTreeMap::new(),
        }
    }

    /// Registers a genesis configuration for a subprotocol.
    ///
    /// # Arguments
    /// * `id` - The subprotocol ID
    /// * `config` - The genesis configuration to register
    pub fn register<T: BorshSerialize>(
        &mut self,
        id: SubprotocolId,
        config: &T,
    ) -> Result<(), AsmError> {
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

    #[derive(Debug, PartialEq, BorshSerialize, BorshDeserialize)]
    struct TestConfig {
        value: u32,
    }

    #[test]
    fn test_genesis_registry() {
        let mut registry = GenesisConfigRegistry::new();
        let config = TestConfig { value: 42 };

        // Register config
        registry.register(1, &config).unwrap();
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
