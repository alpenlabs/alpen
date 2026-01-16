//! Builder for constructing BatchStateDiff from multiple block diffs.

use std::collections::BTreeMap;

use alloy_primitives::U256;
use revm_primitives::{Address, B256};

use super::{AccountChange, AccountDiff, BatchStateDiff, StorageDiff};
use crate::block::{AccountSnapshot, BlockStateDiff};

/// Builder for constructing [`BatchStateDiff`] from consecutive block diffs.
///
/// This builder aggregates state changes across multiple blocks, tracking original
/// values from before the first block to correctly handle:
/// - Created vs Updated distinction (was account None before batch?)
/// - Revert detection (value changed back to original within batch)
/// - Proper nonce delta computation
///
/// # Example
///
/// ```ignore
/// let mut builder = BatchBuilder::new();
/// builder.apply_block(&block1_diff);
/// builder.apply_block(&block2_diff);
/// let diff = builder.build();
/// ```
#[derive(Clone, Debug, Default)]
pub struct BatchBuilder {
    /// Account states: address -> (original_before_batch, current_state).
    /// Original is None if account didn't exist before the batch started.
    accounts: BTreeMap<Address, (Option<AccountSnapshot>, Option<AccountSnapshot>)>,

    /// Storage states: address -> slot -> (original_value, current_value).
    storage: BTreeMap<Address, BTreeMap<U256, (U256, U256)>>,

    /// Deployed contract code hashes (deduplicated).
    deployed_code_hashes: Vec<B256>,
}

impl BatchBuilder {
    /// Creates a new empty builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies a block's state diff. Blocks must be applied in order.
    pub fn apply_block(&mut self, block_diff: &BlockStateDiff) {
        // Process account changes
        for (addr, change) in &block_diff.accounts {
            let entry = self.accounts.entry(*addr).or_insert_with(|| {
                // First time seeing this account - record original from this block
                (change.original.clone(), None)
            });

            // Update current state
            entry.1 = change.current.clone();
        }

        // Process storage changes
        for (addr, storage_diff) in &block_diff.storage {
            let storage_entry = self.storage.entry(*addr).or_default();

            for (slot_key, (original, current)) in &storage_diff.slots {
                let slot_entry = storage_entry.entry(*slot_key).or_insert_with(|| {
                    // First time seeing this slot - record original from this block
                    (*original, *original)
                });
                // Update current value
                slot_entry.1 = *current;
            }
        }

        // Collect deployed contract code hashes
        for hash in &block_diff.deployed_code_hashes {
            if !self.deployed_code_hashes.contains(hash) {
                self.deployed_code_hashes.push(*hash);
            }
        }
    }

    /// Builds the final [`BatchStateDiff`] for DA.
    ///
    /// Filters out accounts/slots that reverted to their original values.
    pub fn build(self) -> BatchStateDiff {
        let mut result = BatchStateDiff::new();

        // Process accounts
        for (addr, (original, current)) in self.accounts {
            let change = match (&original, &current) {
                // Reverted to original or no change
                _ if original == current => continue,
                // Account deleted
                (Some(_), None) => AccountChange::Deleted,
                // Account created
                (None, Some(curr)) => match AccountDiff::from_account_snapshot(curr, None, addr) {
                    Some(diff) => AccountChange::Created(diff),
                    None => continue,
                },
                // Account updated
                (Some(orig), Some(curr)) => {
                    match AccountDiff::from_account_snapshot(curr, Some(orig), addr) {
                        Some(diff) => AccountChange::Updated(diff),
                        None => continue,
                    }
                }
                // Shouldn't happen
                (None, None) => continue,
            };

            result.accounts.insert(addr, change);
        }

        // Process storage
        for (addr, slots) in self.storage {
            let mut storage_diff = StorageDiff::new();

            for (key, (original, current)) in slots {
                // Skip if reverted to original
                if original == current {
                    continue;
                }

                if current.is_zero() {
                    storage_diff.delete_slot(key);
                } else {
                    storage_diff.set_slot(key, current);
                }
            }

            if !storage_diff.is_empty() {
                result.storage.insert(addr, storage_diff);
            }
        }

        result.deployed_code_hashes = self.deployed_code_hashes;
        result
    }
}

// === Single block conversion ===

impl From<&BlockStateDiff> for BatchStateDiff {
    fn from(block_diff: &BlockStateDiff) -> Self {
        let mut builder = BatchBuilder::new();
        builder.apply_block(block_diff);
        builder.build()
    }
}

impl From<BlockStateDiff> for BatchStateDiff {
    fn from(block_diff: BlockStateDiff) -> Self {
        Self::from(&block_diff)
    }
}

#[cfg(test)]
mod tests {
    use revm_primitives::KECCAK_EMPTY;

    use super::*;
    use crate::block::{BlockAccountChange, BlockStorageDiff};

    fn make_account_snapshot(balance: u64, nonce: u64) -> AccountSnapshot {
        AccountSnapshot {
            balance: U256::from(balance),
            nonce,
            code_hash: KECCAK_EMPTY,
        }
    }

    #[test]
    fn test_single_block_created() {
        let mut block = BlockStateDiff::new();
        block.accounts.insert(
            Address::from([0x11u8; 20]),
            BlockAccountChange {
                original: None,
                current: Some(make_account_snapshot(1000, 1)),
            },
        );

        let diff = BatchStateDiff::from(&block);

        assert_eq!(diff.accounts.len(), 1);
        let change = diff.accounts.get(&Address::from([0x11u8; 20])).unwrap();
        assert!(matches!(change, AccountChange::Created(_)));
    }

    #[test]
    fn test_single_block_deleted() {
        let mut block = BlockStateDiff::new();
        block.accounts.insert(
            Address::from([0x11u8; 20]),
            BlockAccountChange {
                original: Some(make_account_snapshot(1000, 1)),
                current: None,
            },
        );

        let diff = BatchStateDiff::from(&block);

        assert_eq!(diff.accounts.len(), 1);
        let change = diff.accounts.get(&Address::from([0x11u8; 20])).unwrap();
        assert!(matches!(change, AccountChange::Deleted));
    }

    #[test]
    fn test_multi_block_revert_detection() {
        let addr = Address::from([0x11u8; 20]);

        // Block 1: balance 0 -> 1000
        let mut block1 = BlockStateDiff::new();
        block1.accounts.insert(
            addr,
            BlockAccountChange {
                original: Some(make_account_snapshot(0, 0)),
                current: Some(make_account_snapshot(1000, 0)),
            },
        );

        // Block 2: balance 1000 -> 0 (revert!)
        let mut block2 = BlockStateDiff::new();
        block2.accounts.insert(
            addr,
            BlockAccountChange {
                original: Some(make_account_snapshot(1000, 0)),
                current: Some(make_account_snapshot(0, 0)),
            },
        );

        let mut builder = BatchBuilder::new();
        builder.apply_block(&block1);
        builder.apply_block(&block2);
        let diff = builder.build();

        // Should detect revert and exclude this account
        assert!(diff.accounts.is_empty());
    }

    #[test]
    fn test_multi_block_storage_revert() {
        let addr = Address::from([0x11u8; 20]);
        let slot = U256::from(1);

        // Block 1: slot 0 -> 100
        let mut block1 = BlockStateDiff::new();
        let mut storage1 = BlockStorageDiff::new();
        storage1.slots.insert(slot, (U256::ZERO, U256::from(100)));
        block1.storage.insert(addr, storage1);

        // Block 2: slot 100 -> 0 (revert!)
        let mut block2 = BlockStateDiff::new();
        let mut storage2 = BlockStorageDiff::new();
        storage2.slots.insert(slot, (U256::from(100), U256::ZERO));
        block2.storage.insert(addr, storage2);

        let mut builder = BatchBuilder::new();
        builder.apply_block(&block1);
        builder.apply_block(&block2);
        let diff = builder.build();

        // Should detect revert and exclude this storage change
        assert!(diff.storage.is_empty());
    }

    #[test]
    fn test_multi_block_cumulative_nonce() {
        let addr = Address::from([0x11u8; 20]);

        // Block 1: nonce 0 -> 2
        let mut block1 = BlockStateDiff::new();
        block1.accounts.insert(
            addr,
            BlockAccountChange {
                original: Some(make_account_snapshot(1000, 0)),
                current: Some(make_account_snapshot(1000, 2)),
            },
        );

        // Block 2: nonce 2 -> 5
        let mut block2 = BlockStateDiff::new();
        block2.accounts.insert(
            addr,
            BlockAccountChange {
                original: Some(make_account_snapshot(1000, 2)),
                current: Some(make_account_snapshot(1000, 5)),
            },
        );

        let mut builder = BatchBuilder::new();
        builder.apply_block(&block1);
        builder.apply_block(&block2);
        let diff = builder.build();

        // Total nonce delta should be 5 (from original 0 to final 5)
        let change = diff.accounts.get(&addr).unwrap();
        if let AccountChange::Updated(account_diff) = change {
            assert_eq!(account_diff.nonce.diff(), Some(&5));
        } else {
            panic!("Expected Updated");
        }
    }

    #[test]
    fn test_code_hash_deduplication() {
        let hash = B256::from([0x11u8; 32]);

        let mut block1 = BlockStateDiff::new();
        block1.deployed_code_hashes.push(hash);

        let mut block2 = BlockStateDiff::new();
        block2.deployed_code_hashes.push(hash); // Same hash

        let mut builder = BatchBuilder::new();
        builder.apply_block(&block1);
        builder.apply_block(&block2);
        let diff = builder.build();

        // Should be deduplicated
        assert_eq!(diff.deployed_code_hashes.len(), 1);
    }
}
