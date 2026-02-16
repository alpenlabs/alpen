//! Builder for constructing BatchStateDiff from multiple block diffs.

use std::collections::BTreeMap;

use alloy_primitives::{Bytes, U256};
use revm_primitives::{Address, B256};

use super::{AccountChange, AccountDiff, BatchStateDiff, StorageDiff};
use crate::block::{AccountSnapshot, BlockStateChanges};

/// Tracks the original and current state of a value across a batch.
///
/// Used internally by [`BatchBuilder`] to detect reverts (when current == original)
/// and compute proper diffs.
#[derive(Clone, Debug, Default)]
struct TrackedState<T> {
    /// State before the batch started.
    original: T,
    /// Current state after applying blocks.
    current: T,
}

impl<T: Clone> TrackedState<T> {
    /// Creates a new tracked state where both original and current are the same.
    fn new(value: T) -> Self {
        Self {
            original: value.clone(),
            current: value,
        }
    }
}

impl<T> TrackedState<T> {
    /// Returns true if the value reverted to its original state.
    fn is_unchanged(&self) -> bool
    where
        T: PartialEq,
    {
        self.original == self.current
    }
}

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
///
/// # Limitations
///
/// ## No block ordering validation
///
/// Currently, [`apply_block`](Self::apply_block) does not validate that blocks are applied
/// in consecutive order. Callers must ensure blocks are applied sequentially (block N, then
/// block N+1, etc.). Applying blocks out of order will produce incorrect diffs.
///
/// **TODO**: Add block number tracking and validation to reject non-consecutive blocks.
///
/// ## No pre-state provider
///
/// The builder assumes the first block diff for each account/slot contains the correct
/// "original" value. If an account is touched in block 5 but not in blocks 1-4, the builder
/// has no way to know the account's state before block 1.
///
/// This works correctly when:
/// - Building a diff for a single block (original comes from that block's diff)
/// - All touched accounts/slots appear in at least the first block where they're modified
///
/// This may produce incorrect "Created vs Updated" classification when:
/// - An account existed before the batch but is first touched in a later block
///
/// **TODO**: Add an optional pre-state provider trait that the builder can query for the
/// true original state of any account/slot when first encountered within the batch.
#[derive(Clone, Debug, Default)]
pub struct BatchBuilder {
    /// Account states: address -> tracked state (original is None if account didn't exist).
    accounts: BTreeMap<Address, TrackedState<Option<AccountSnapshot>>>,

    /// Storage states: address -> slot -> tracked value.
    storage: BTreeMap<Address, BTreeMap<U256, TrackedState<U256>>>,

    /// Deployed contract bytecodes keyed by code hash (deduplicated).
    deployed_bytecodes: BTreeMap<B256, Bytes>,
}

impl BatchBuilder {
    /// Creates a new empty builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies a block's state diff. Blocks must be applied in order.
    pub fn apply_block(&mut self, block_diff: &BlockStateChanges) {
        // Process account changes
        for (addr, change) in &block_diff.accounts {
            let entry = self.accounts.entry(*addr).or_insert_with(|| {
                // First time seeing this account - record original from this block
                TrackedState {
                    original: change.original.clone(),
                    current: None,
                }
            });
            entry.current = change.current.clone();
        }

        // Process storage changes
        for (addr, storage_diff) in &block_diff.storage {
            let storage_entry = self.storage.entry(*addr).or_default();

            for (slot_key, (original, current)) in &storage_diff.slots {
                let slot_entry = storage_entry.entry(*slot_key).or_insert_with(|| {
                    // First time seeing this slot - record original from this block
                    TrackedState::new(*original)
                });
                slot_entry.current = *current;
            }
        }

        // Collect deployed contract bytecodes (BTreeMap handles deduplication by hash)
        self.deployed_bytecodes.extend(
            block_diff
                .deployed_bytecodes
                .iter()
                .map(|(k, v)| (*k, v.clone())),
        );
    }

    /// Builds the final [`BatchStateDiff`] for DA.
    ///
    /// Filters out accounts/slots that reverted to their original values.
    pub fn build(self) -> BatchStateDiff {
        let mut result = BatchStateDiff::new();

        // Process accounts
        for (addr, tracked) in self.accounts {
            // Skip if reverted to original
            if tracked.is_unchanged() {
                continue;
            }

            let change = match (&tracked.original, &tracked.current) {
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
                // Shouldn't happen (caught by is_unchanged)
                (None, None) => continue,
            };

            result.accounts.insert(addr, change);
        }

        // Process storage
        for (addr, slots) in self.storage {
            let mut storage_diff = StorageDiff::new();

            for (key, tracked) in slots {
                // Skip if reverted to original
                if tracked.is_unchanged() {
                    continue;
                }

                if tracked.current.is_zero() {
                    storage_diff.delete_slot(key);
                } else {
                    storage_diff.set_slot(key, tracked.current);
                }
            }

            if !storage_diff.is_empty() {
                result.storage.insert(addr, storage_diff);
            }
        }

        result.deployed_bytecodes = self.deployed_bytecodes;
        result
    }
}

// === Single block conversion ===

impl From<&BlockStateChanges> for BatchStateDiff {
    fn from(block_diff: &BlockStateChanges) -> Self {
        let mut builder = BatchBuilder::new();
        builder.apply_block(block_diff);
        builder.build()
    }
}

impl From<BlockStateChanges> for BatchStateDiff {
    fn from(block_diff: BlockStateChanges) -> Self {
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
        let mut block = BlockStateChanges::new();
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
        let mut block = BlockStateChanges::new();
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
        let mut block1 = BlockStateChanges::new();
        block1.accounts.insert(
            addr,
            BlockAccountChange {
                original: Some(make_account_snapshot(0, 0)),
                current: Some(make_account_snapshot(1000, 0)),
            },
        );

        // Block 2: balance 1000 -> 0 (revert!)
        let mut block2 = BlockStateChanges::new();
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
        let mut block1 = BlockStateChanges::new();
        let mut storage1 = BlockStorageDiff::new();
        storage1.slots.insert(slot, (U256::ZERO, U256::from(100)));
        block1.storage.insert(addr, storage1);

        // Block 2: slot 100 -> 0 (revert!)
        let mut block2 = BlockStateChanges::new();
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
        let mut block1 = BlockStateChanges::new();
        block1.accounts.insert(
            addr,
            BlockAccountChange {
                original: Some(make_account_snapshot(1000, 0)),
                current: Some(make_account_snapshot(1000, 2)),
            },
        );

        // Block 2: nonce 2 -> 5
        let mut block2 = BlockStateChanges::new();
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
            assert_eq!(account_diff.nonce.diff().and_then(|v| v.to_i64()), Some(5));
        } else {
            panic!("Expected Updated");
        }
    }

    #[test]
    fn test_bytecode_deduplication() {
        let hash = B256::from([0x11u8; 32]);
        let bytecode = Bytes::from_static(&[0x60, 0x80, 0x60, 0x40]);

        let mut block1 = BlockStateChanges::new();
        block1.deployed_bytecodes.insert(hash, bytecode.clone());

        let mut block2 = BlockStateChanges::new();
        block2.deployed_bytecodes.insert(hash, bytecode); // Same hash

        let mut builder = BatchBuilder::new();
        builder.apply_block(&block1);
        builder.apply_block(&block2);
        let diff = builder.build();

        // Should be deduplicated by hash
        assert_eq!(diff.deployed_bytecodes.len(), 1);
        assert!(diff.deployed_bytecodes.contains_key(&hash));
    }
}
