//! Per-block account types.

use alloy_primitives::U256;
use revm_primitives::{B256, KECCAK_EMPTY};
use serde::{Deserialize, Serialize};
use strata_mpt::StateAccount;

/// Point-in-time snapshot of account state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountSnapshot {
    pub balance: U256,
    pub nonce: u64,
    pub code_hash: B256,
}

impl Default for AccountSnapshot {
    fn default() -> Self {
        Self {
            balance: U256::ZERO,
            nonce: 0,
            code_hash: KECCAK_EMPTY,
        }
    }
}

impl From<&StateAccount> for AccountSnapshot {
    fn from(acc: &StateAccount) -> Self {
        Self {
            balance: acc.balance,
            nonce: acc.nonce,
            code_hash: acc.code_hash,
        }
    }
}

/// Account change with original state for tracking across blocks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockAccountChange {
    /// Original state before this block (None if account didn't exist).
    pub original: Option<AccountSnapshot>,
    /// Current state after this block (None if account was deleted).
    pub current: Option<AccountSnapshot>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_account_snapshot_roundtrip() {
        let state = AccountSnapshot {
            balance: U256::from(1000),
            nonce: 5,
            code_hash: B256::from([0x11u8; 32]),
        };

        let encoded = bincode::serialize(&state).unwrap();
        let decoded: AccountSnapshot = bincode::deserialize(&encoded).unwrap();

        assert_eq!(decoded, state);
    }

    #[test]
    fn test_block_account_change_created() {
        let change = BlockAccountChange {
            original: None,
            current: Some(AccountSnapshot {
                balance: U256::from(500),
                nonce: 1,
                code_hash: B256::ZERO,
            }),
        };

        let encoded = bincode::serialize(&change).unwrap();
        let decoded: BlockAccountChange = bincode::deserialize(&encoded).unwrap();

        assert_eq!(decoded, change);
    }

    #[test]
    fn test_block_account_change_deleted() {
        let change = BlockAccountChange {
            original: Some(AccountSnapshot {
                balance: U256::from(500),
                nonce: 1,
                code_hash: B256::ZERO,
            }),
            current: None,
        };

        let encoded = bincode::serialize(&change).unwrap();
        let decoded: BlockAccountChange = bincode::deserialize(&encoded).unwrap();

        assert_eq!(decoded, change);
    }
}
