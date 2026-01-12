//! Builder for constructing DaEeStateDiff from BundleState.

use std::collections::BTreeMap;

use alloy_primitives::U256;
use revm::database::{BundleAccount, BundleState};
use revm_primitives::{Address, B256, KECCAK_EMPTY};
use strata_da_framework::DaRegister;

use crate::{
    account::{DaAccountChange, DaAccountDiff},
    codec::{CodecB256, CodecU256},
    diff::DaEeStateDiff,
    storage::DaAccountStorageDiff,
};

/// Builder for `DaEeStateDiff` that works directly with `BundleState`.
///
/// This builder aggregates state changes across multiple blocks, tracking original
/// values to correctly handle:
/// - Created vs Updated distinction (was account None before batch?)
/// - Revert detection (value changed back to original within batch)
/// - Proper nonce delta computation
#[derive(Clone, Debug, Default)]
pub struct DaEeStateDiffBuilder {
    /// Current account states: address -> (original_before_batch, current_state)
    /// Original is None if account didn't exist before the batch started.
    accounts: BTreeMap<Address, (Option<AccountSnapshot>, Option<AccountSnapshot>)>,

    /// Current storage states: address -> slot -> (original_value, current_value)
    storage: BTreeMap<Address, BTreeMap<U256, (U256, U256)>>,

    /// Deployed contract code hashes (deduplicated).
    deployed_code_hashes: Vec<B256>,
}

/// Snapshot of account state for tracking changes.
#[derive(Clone, Debug, PartialEq, Eq)]
struct AccountSnapshot {
    balance: U256,
    nonce: u64,
    code_hash: B256,
}

impl AccountSnapshot {
    fn from_bundle_account(acc: &BundleAccount) -> Option<Self> {
        acc.info.as_ref().map(|info| Self {
            balance: info.balance,
            nonce: info.nonce,
            code_hash: info.code_hash,
        })
    }
}

impl DaEeStateDiffBuilder {
    /// Creates a new empty builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies a `BundleState` from a single block.
    ///
    /// Can be called multiple times to aggregate changes across blocks.
    pub fn apply_bundle(&mut self, bundle: &BundleState) {
        // Process account changes
        for (addr, bundle_acc) in &bundle.state {
            let original_snapshot = bundle_acc
                .original_info
                .as_ref()
                .map(|info| AccountSnapshot {
                    balance: info.balance,
                    nonce: info.nonce,
                    code_hash: info.code_hash,
                });
            let current_snapshot = AccountSnapshot::from_bundle_account(bundle_acc);

            // Get or initialize the tracking entry
            let entry = self.accounts.entry(*addr).or_insert_with(|| {
                // First time seeing this account in the batch - record original
                (original_snapshot.clone(), None)
            });

            // Update current state
            entry.1 = current_snapshot;

            // Process storage changes for this account
            for (slot_key, slot) in &bundle_acc.storage {
                let storage_entry = self.storage.entry(*addr).or_default();
                let slot_entry = storage_entry.entry(*slot_key).or_insert_with(|| {
                    // First time seeing this slot - record original
                    (
                        slot.previous_or_original_value,
                        slot.previous_or_original_value,
                    )
                });
                // Update current value
                slot_entry.1 = slot.present_value;
            }
        }

        // Collect deployed contract code hashes
        for bytecode in bundle.contracts.values() {
            let code_hash = bytecode.hash_slow();
            if code_hash != KECCAK_EMPTY && !self.deployed_code_hashes.contains(&code_hash) {
                self.deployed_code_hashes.push(code_hash);
            }
        }
    }

    /// Builds the final `DaEeStateDiff`.
    ///
    /// Filters out accounts/slots that reverted to their original values.
    pub fn build(self) -> DaEeStateDiff {
        let mut result = DaEeStateDiff::new();

        // Process accounts
        for (addr, (original, current)) in self.accounts {
            // Skip if current equals original (reverted within batch)
            if original == current {
                continue;
            }

            let change = match (original, current) {
                (None, Some(curr)) => {
                    // Account was created
                    DaAccountChange::Created(DaAccountDiff {
                        balance: DaRegister::new_set(CodecU256(curr.balance)),
                        nonce_incr: if curr.nonce > 0 {
                            Some(curr.nonce.min(u8::MAX as u64) as u8)
                        } else {
                            None
                        },
                        code_hash: if curr.code_hash != KECCAK_EMPTY {
                            DaRegister::new_set(CodecB256(curr.code_hash))
                        } else {
                            DaRegister::new_unset()
                        },
                    })
                }
                (Some(orig), Some(curr)) => {
                    // Account was updated
                    let balance = if orig.balance != curr.balance {
                        DaRegister::new_set(CodecU256(curr.balance))
                    } else {
                        DaRegister::new_unset()
                    };

                    let nonce_delta = curr.nonce.saturating_sub(orig.nonce);
                    let nonce_incr = if nonce_delta > 0 {
                        Some(nonce_delta.min(u8::MAX as u64) as u8)
                    } else {
                        None
                    };

                    let code_hash = if orig.code_hash != curr.code_hash {
                        DaRegister::new_set(CodecB256(curr.code_hash))
                    } else {
                        DaRegister::new_unset()
                    };

                    let diff = DaAccountDiff {
                        balance,
                        nonce_incr,
                        code_hash,
                    };

                    // Skip if no actual changes
                    if diff.is_unchanged() {
                        continue;
                    }

                    DaAccountChange::Updated(diff)
                }
                (Some(_), None) => {
                    // Account was deleted
                    DaAccountChange::Deleted
                }
                (None, None) => {
                    // No change (shouldn't happen, but handle gracefully)
                    continue;
                }
            };

            result.accounts.insert(addr, change);
        }

        // Process storage
        for (addr, slots) in self.storage {
            let mut storage_diff = DaAccountStorageDiff::new();

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

impl From<&BundleState> for DaEeStateDiff {
    fn from(bundle: &BundleState) -> Self {
        let mut builder = DaEeStateDiffBuilder::new();
        builder.apply_bundle(bundle);
        builder.build()
    }
}

impl From<BundleState> for DaEeStateDiff {
    fn from(bundle: BundleState) -> Self {
        Self::from(&bundle)
    }
}
