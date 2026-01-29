//! Account diff types.

use strata_da_framework::{
    DaCounter, DaWrite, counter_schemes::CtrU64BySignedVarint, make_compound_impl,
};

use super::snark::{SnarkAccountDiff, SnarkAccountTarget};

/// Per-account diff keyed by account type.
///
/// The account type is implied by pre-state; the snark field is only populated
/// for snark accounts.
#[derive(Debug)]
pub struct AccountDiff {
    /// Balance: The account’s balance in satoshis. The `SignedVarint` increment
    /// guarantees that small increments (most common) will take minimal space
    /// while allowing for maximum bitcoin supply (21 million BTC) changes
    /// (very unlikely).
    pub balance: DaCounter<CtrU64BySignedVarint>,

    /// Snark state diff.
    pub snark: SnarkAccountDiff,
}

impl Default for AccountDiff {
    fn default() -> Self {
        Self {
            balance: DaCounter::new_unchanged(),
            snark: SnarkAccountDiff::default(),
        }
    }
}

impl AccountDiff {
    /// Creates a new account diff.
    pub fn new(balance: DaCounter<CtrU64BySignedVarint>, snark: SnarkAccountDiff) -> Self {
        Self { balance, snark }
    }

    /// Returns the balance diff, regardless of account type.
    pub fn balance(&self) -> &DaCounter<CtrU64BySignedVarint> {
        &self.balance
    }

    pub fn is_default(&self) -> bool {
        DaWrite::is_default(self)
    }
}

make_compound_impl! {
    AccountDiff < (), crate::DaError > u8 => AccountDiffTarget {
        balance: counter (CtrU64BySignedVarint),
        snark: compound (SnarkAccountDiff),
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AccountDiffTarget {
    pub balance: u64,
    pub snark: SnarkAccountTarget,
}
