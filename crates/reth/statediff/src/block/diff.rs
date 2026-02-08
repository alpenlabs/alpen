//! Per-block state diff for DB storage.

use std::collections::BTreeMap;

use alloy_primitives::Bytes;
use revm::database::BundleState;
use revm_primitives::{Address, B256, KECCAK_EMPTY};
use serde::{Deserialize, Serialize};

use super::{AccountSnapshot, BlockAccountChange, BlockStorageDiff};

/// Per-block state diff stored in DB by exex.
///
/// Contains both original and current values to enable proper batch aggregation
/// with revert detection when building [`BatchStateDiff`](crate::batch::BatchStateDiff).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BlockStateChanges {
    /// Account changes with original values for tracking.
    pub accounts: BTreeMap<Address, BlockAccountChange>,
    /// Storage changes with original values per account.
    pub storage: BTreeMap<Address, BlockStorageDiff>,
    /// Deployed contract bytecodes keyed by code hash.
    /// This ensures bytecode is available for DA reconstruction.
    pub deployed_bytecodes: BTreeMap<B256, Bytes>,
}

impl BlockStateChanges {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.accounts.is_empty() && self.storage.is_empty() && self.deployed_bytecodes.is_empty()
    }
}

// === Conversion from BundleState ===

impl From<&BundleState> for BlockStateChanges {
    fn from(bundle: &BundleState) -> Self {
        let mut result = Self::new();

        // Process account changes
        for (addr, bundle_acc) in &bundle.state {
            let original = bundle_acc
                .original_info
                .as_ref()
                .map(|info| AccountSnapshot {
                    balance: info.balance,
                    nonce: info.nonce,
                    code_hash: info.code_hash,
                });

            let current = bundle_acc.info.as_ref().map(|info| AccountSnapshot {
                balance: info.balance,
                nonce: info.nonce,
                code_hash: info.code_hash,
            });

            // Only include if there's an actual change
            if original != current {
                result
                    .accounts
                    .insert(*addr, BlockAccountChange { original, current });
            }

            // Process storage changes
            let mut storage_diff = BlockStorageDiff::new();
            for (slot_key, slot) in &bundle_acc.storage {
                let original_value = slot.previous_or_original_value;
                let current_value = slot.present_value;

                if original_value != current_value {
                    storage_diff
                        .slots
                        .insert(*slot_key, (original_value, current_value));
                }
            }

            if !storage_diff.is_empty() {
                result.storage.insert(*addr, storage_diff);
            }
        }

        // Collect deployed contract bytecodes (keyed by hash for deduplication)
        for bytecode in bundle.contracts.values() {
            let code_hash = bytecode.hash_slow();
            if code_hash != KECCAK_EMPTY && !result.deployed_bytecodes.contains_key(&code_hash) {
                result
                    .deployed_bytecodes
                    .insert(code_hash, Bytes::from(bytecode.original_bytes().to_vec()));
            }
        }

        result
    }
}

impl From<BundleState> for BlockStateChanges {
    fn from(bundle: BundleState) -> Self {
        Self::from(&bundle)
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::U256;

    use super::*;

    #[test]
    fn test_block_state_diff_roundtrip() {
        let mut diff = BlockStateChanges::new();

        diff.accounts.insert(
            Address::from([0x11u8; 20]),
            BlockAccountChange {
                original: None,
                current: Some(AccountSnapshot {
                    balance: U256::from(1000),
                    nonce: 1,
                    code_hash: B256::from([0x22u8; 32]),
                }),
            },
        );

        let mut storage = BlockStorageDiff::new();
        storage
            .slots
            .insert(U256::from(1), (U256::ZERO, U256::from(100)));
        diff.storage.insert(Address::from([0x11u8; 20]), storage);

        diff.deployed_bytecodes
            .insert(B256::from([0x33u8; 32]), Bytes::from_static(&[0x60, 0x80]));

        let encoded = bincode::serialize(&diff).unwrap();
        let decoded: BlockStateChanges = bincode::deserialize(&encoded).unwrap();

        assert_eq!(decoded.accounts.len(), 1);
        assert_eq!(decoded.storage.len(), 1);
        assert_eq!(decoded.deployed_bytecodes.len(), 1);
    }
}
