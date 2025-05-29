use alloy_primitives::U256;
use revm::db::{BundleAccount, StorageWithOriginalValues};
use revm_primitives::{AccountInfo, B256};
use serde::{Deserialize, Serialize};

/// An ethereum account representation suitable for state diffs.
///
/// P.S. Storage data is stripped from here and stored separately.
#[derive(Clone, Default, Debug, Deserialize, Serialize, PartialEq)]
#[serde(into = "AccountTuple", from = "AccountTuple")]
pub struct Account {
    /// Account balance.
    pub balance: U256,
    /// Account nonce.
    pub nonce: u64,
    /// code hash,
    pub code_hash: B256,
}

/// Tuple representation of [`Account`] for serde for efficient storage.
type AccountTuple = (U256, u64, B256);

/// Changes made to a single account with original values.
#[derive(Clone, Default, Debug, Deserialize, Serialize, PartialEq)]
#[serde(into = "AccountChangesTuple", from = "AccountChangesTuple")]
pub struct AccountChanges {
    pub original_info: Option<Account>,
    pub present_info: Option<Account>,
    pub storage: StorageWithOriginalValues,
}

/// Tuple representation of [`AccountChanges`] for serde for efficient storage.
type AccountChangesTuple = (Option<Account>, Option<Account>, StorageWithOriginalValues);

impl Account {
    pub const fn new(balance: U256, nonce: u64, code_hash: B256) -> Self {
        Self {
            balance,
            nonce,
            code_hash,
        }
    }
}

impl From<AccountInfo> for Account {
    fn from(value: AccountInfo) -> Self {
        Self {
            balance: value.balance,
            nonce: value.nonce,
            code_hash: value.code_hash,
        }
    }
}

impl From<Account> for AccountTuple {
    fn from(account: Account) -> Self {
        (account.balance, account.nonce, account.code_hash)
    }
}

impl From<AccountTuple> for Account {
    fn from((balance, nonce, code_hash): AccountTuple) -> Self {
        Self {
            balance,
            nonce,
            code_hash,
        }
    }
}

impl AccountChanges {
    pub const fn new(
        original_info: Option<Account>,
        present_info: Option<Account>,
        storage: StorageWithOriginalValues,
    ) -> Self {
        Self {
            original_info,
            present_info,
            storage,
        }
    }
}

impl From<BundleAccount> for AccountChanges {
    fn from(value: BundleAccount) -> Self {
        Self {
            present_info: value.info.map(Account::from),
            original_info: value.original_info.map(Account::from),
            storage: value.storage,
        }
    }
}

impl From<AccountChanges> for AccountChangesTuple {
    fn from(ac: AccountChanges) -> Self {
        (ac.original_info, ac.present_info, ac.storage)
    }
}

impl From<AccountChangesTuple> for AccountChanges {
    fn from((original_info, present_info, storage): AccountChangesTuple) -> Self {
        Self {
            original_info,
            present_info,
            storage,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use alloy_primitives::{B256, U256};
    use revm::db::states::StorageSlot;
    use revm_primitives::hex;
    use serde_json;

    use super::*;

    fn dummy_account(balance: u64, nonce: u64) -> Account {
        Account {
            balance: U256::from(balance),
            nonce,
            code_hash: B256::from([0x11; 32]),
        }
    }

    #[test]
    fn test_account_serde() {
        let acc = dummy_account(100, 1);
        let json = serde_json::to_string(&acc).unwrap();
        assert_eq!(
            json,
            format!(
                "[\"0x{:x}\",{},\"0x{}\"]",
                acc.balance,
                acc.nonce,
                hex::encode(acc.code_hash)
            )
        );

        let deserialized: Account = serde_json::from_str(&json).unwrap();
        assert_eq!(acc, deserialized);
    }

    #[test]
    fn test_account_changes_serde() {
        let mut map = HashMap::with_hasher(alloy_primitives::map::DefaultHashBuilder::default());
        map.insert(
            U256::from(147),
            StorageSlot::new_changed(U256::from(1), U256::from(2)),
        );

        let changes = AccountChanges {
            original_info: Some(dummy_account(50, 5)),
            present_info: Some(dummy_account(75, 6)),
            storage: map,
        };

        let json = serde_json::to_string(&changes).unwrap();
        let deserialized: AccountChanges = serde_json::from_str(&json).unwrap();
        assert_eq!(changes, deserialized);
    }

    #[test]
    fn test_account_changes_with_none_serde() {
        let changes = AccountChanges {
            original_info: None,
            present_info: None,
            storage: Default::default(),
        };

        let json = serde_json::to_string(&changes).unwrap();
        assert_eq!(json, "[null,null,{}]");

        let deserialized: AccountChanges = serde_json::from_str(&json).unwrap();
        assert_eq!(changes, deserialized);
    }
}
