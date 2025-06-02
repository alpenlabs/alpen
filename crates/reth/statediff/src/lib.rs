//! State diff for the reth node.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use std::collections::hash_map::Entry;

use account::Account;
use alloy_primitives::U256;
use revm_primitives::{Address, Bytecode, HashMap, HashSet};

pub mod account;
pub mod block;
pub mod state;
pub use block::{BatchStateDiff, BlockStateDiff};

/// A builder for the [`BatchStateDiff`].
///
/// N.B. "original" counterparts are needed to correctly construct the [`BatchStateDiff`]
/// for the range of blocks and not include changes for the keys whose values
/// were changed inside batch, but end up having the same value as before the batch.
#[derive(Clone, Default, Debug)]
pub struct BatchStateDiffBuilder {
    /// An account representation in the diff.
    /// [`Option::None`] indicates the account was destructed, but existed before the batch.
    pub accounts: HashMap<Address, Option<Account>>,

    /// The *FIRST* original [`Account`] for the account in the batch.
    /// [`Option::None`] if the account was absent before the batch.
    pub original_accounts: HashMap<Address, Option<Account>>,

    /// A collection of deployed smart contracts within the batch.
    pub contracts: HashSet<Bytecode>,

    /// Storage slots.
    pub storage_slots: HashMap<Address, HashMap<U256, U256>>,

    /// The *FIRST* original value for the slot in the batch.
    pub original_storage_slots: HashMap<Address, HashMap<U256, U256>>,
}

impl BatchStateDiffBuilder {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn apply(&mut self, diff: BlockStateDiff) {
        self.contracts.extend(diff.contracts);

        for (addr, acc) in diff.state {
            // Update original account if seen for the first time in the batch.
            if let Entry::Vacant(e) = self.original_accounts.entry(addr) {
                e.insert(acc.original_info);
            }

            // Now, modify the actual account entry.
            let cur_account = acc.present_info;
            if cur_account == *self.original_accounts.get(&addr).unwrap() {
                // Remove if the actual account equals to the original.
                self.accounts.remove(&addr);
            } else {
                // The current account is different, update it.
                self.accounts.insert(addr, cur_account);
            }

            let original_acc_storage = self.original_storage_slots.entry(addr).or_default();
            let cur_acc_storage = self.storage_slots.entry(addr).or_default();

            for (key, value) in &acc.storage {
                // Update original account storage if seen for the first time in the batch.
                if !original_acc_storage.contains_key(key) {
                    original_acc_storage.insert(*key, value.original_value());
                }

                // Now, modify the actual account storage slots.
                let cur_storage_value = value.present_value();
                if &cur_storage_value == original_acc_storage.get(key).unwrap() {
                    // Remove if the actual value equals to the original.
                    cur_acc_storage.remove(key);
                } else {
                    // The current value is different, update it.
                    cur_acc_storage.insert(*key, cur_storage_value);
                }
            }

            // It can happen that the storage slots for a certain account has been completely
            // reverted, so remove the account from the map.
            if cur_acc_storage.is_empty() {
                self.storage_slots.remove(&addr);
            }
        }
    }

    pub fn build(self) -> BatchStateDiff {
        self.into()
    }
}

impl From<BatchStateDiffBuilder> for BatchStateDiff {
    fn from(value: BatchStateDiffBuilder) -> Self {
        BatchStateDiff {
            accounts: value.accounts,
            contracts: value.contracts,
            storage_slots: value.storage_slots,
        }
    }
}

#[cfg(test)]
mod tests {
    use revm::db::states::StorageSlot;
    use revm_primitives::{
        alloy_primitives::map::HashMap, map::DefaultHashBuilder, Address, HashSet, KECCAK_EMPTY,
        U256,
    };

    use crate::{
        account::{Account, AccountChanges},
        block::{BatchStateDiff, BlockStateDiff},
        BatchStateDiffBuilder,
    };

    fn acc_addr1() -> Address {
        Address::new([0x60; 20])
    }

    fn acc_addr2() -> Address {
        Address::new([0x61; 20])
    }

    fn acc_info1() -> Account {
        Account {
            nonce: 1,
            balance: U256::from(10),
            code_hash: KECCAK_EMPTY,
        }
    }

    fn acc_info2() -> Account {
        Account {
            nonce: 3,
            balance: U256::from(20),
            code_hash: KECCAK_EMPTY,
        }
    }

    fn slot1() -> U256 {
        U256::from(5)
    }

    fn slot2() -> U256 {
        U256::from(7)
    }

    fn slot_changes() -> HashMap<U256, StorageSlot> {
        HashMap::from_iter([
            (
                slot1(),
                StorageSlot::new_changed(U256::from(0), U256::from(10)),
            ),
            (
                slot2(),
                StorageSlot::new_changed(U256::from(10), U256::from(15)),
            ),
        ])
    }

    fn revert_slot_changes(
        slot_changes: &HashMap<U256, StorageSlot>,
    ) -> HashMap<U256, StorageSlot> {
        HashMap::<U256, StorageSlot, DefaultHashBuilder>::from_iter(slot_changes.iter().map(
            |(k, v)| {
                (
                    *k,
                    StorageSlot::new_changed(v.present_value, v.previous_or_original_value),
                )
            },
        ))
    }

    fn test_state_diff_acc1() -> BlockStateDiff {
        let mut test_diff = BlockStateDiff {
            state: HashMap::default(),
            contracts: HashSet::default(),
        };

        test_diff.state.insert(
            acc_addr1(),
            AccountChanges::new(None, Some(acc_info1()), slot_changes()),
        );

        test_diff
    }

    fn test_state_diff_acc_old_slots1() -> BlockStateDiff {
        let mut test_diff = BlockStateDiff {
            state: HashMap::default(),
            contracts: HashSet::default(),
        };

        test_diff.state.insert(
            acc_addr1(),
            AccountChanges::new(
                Some(acc_info1()),
                Some(Account {
                    nonce: 2,
                    balance: U256::from(9),
                    code_hash: KECCAK_EMPTY,
                }),
                revert_slot_changes(&slot_changes()),
            ),
        );

        test_diff
    }

    fn test_state_diff_acc2() -> BlockStateDiff {
        let mut test_diff = BlockStateDiff {
            state: HashMap::default(),
            contracts: HashSet::default(),
        };

        test_diff.state.insert(
            acc_addr2(),
            AccountChanges::new(None, Some(acc_info2()), HashMap::default()),
        );

        test_diff
    }

    #[test]
    fn basic_batch_state_diff() {
        let mut batch_diff = BatchStateDiffBuilder::new();
        batch_diff.apply(test_state_diff_acc1());
        batch_diff.apply(test_state_diff_acc2());
        let batch_diff: BatchStateDiff = batch_diff.build();

        let acc1 = batch_diff
            .accounts
            .get(&acc_addr1())
            .expect("account1 should be present")
            .clone();
        let info1 = acc1.unwrap();
        assert!(info1 == acc_info1());

        let acc2 = batch_diff
            .accounts
            .get(&acc_addr2())
            .expect("account2 should be present")
            .clone();
        let info2 = acc2.unwrap();
        assert!(info2 == acc_info2());

        assert!(batch_diff.storage_slots.contains_key(&acc_addr1()));
    }

    #[test]
    fn multiple_slot_writes() {
        let mut batch_diff = BatchStateDiffBuilder::new();
        batch_diff.apply(test_state_diff_acc1());
        batch_diff.apply(test_state_diff_acc_old_slots1());

        let acc1 = batch_diff
            .accounts
            .get(&acc_addr1())
            .expect("account1 should be present")
            .clone();
        let info1 = acc1.unwrap();
        let expected_info = Account {
            nonce: 2,
            balance: U256::from(9),
            code_hash: KECCAK_EMPTY,
        };
        assert!(info1 == expected_info);
        // Slots were reverted to initial values, so the map should be empty.
        assert!(batch_diff.storage_slots.is_empty());

        // Apply slots again and check it was recorded.
        batch_diff.apply(test_state_diff_acc1());
        let batch_diff: BatchStateDiff = batch_diff.build();

        assert!(
            batch_diff
                .storage_slots
                .get(&acc_addr1())
                .unwrap()
                .get(&slot1())
                .unwrap()
                == &slot_changes().get(&slot1()).unwrap().present_value()
        )
    }

    #[test]
    fn smart_contract_diff() {
        let mut test_diff = BlockStateDiff {
            state: HashMap::default(),
            contracts: HashSet::default(),
        };
        test_diff
            .contracts
            .insert(revm_primitives::Bytecode::LegacyRaw(b"123".into()));

        let mut batch_diff = BatchStateDiffBuilder::new();
        batch_diff.apply(test_diff);
        let batch_diff: BatchStateDiff = batch_diff.build();

        assert!(!batch_diff.contracts.is_empty())
    }
}
