//! Account diff types.

use strata_da_framework::{
    DaCounter, DaWrite, counter_schemes::CtrU64BySignedVarInt, make_compound_impl,
};

use super::snark::{SnarkAccountDiff, SnarkAccountTarget};

/// Per-account diff keyed by account type.
///
/// The account type is implied by pre-state; the snark field is only populated
/// for snark accounts.
#[derive(Debug)]
pub struct AccountDiff {
    /// Balance counter diff (signed delta in satoshis).
    pub balance: DaCounter<CtrU64BySignedVarInt>,

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
    pub fn new(balance: DaCounter<CtrU64BySignedVarInt>, snark: SnarkAccountDiff) -> Self {
        Self { balance, snark }
    }

    /// Returns the balance diff, regardless of account type.
    pub fn balance(&self) -> &DaCounter<CtrU64BySignedVarInt> {
        &self.balance
    }

    pub fn is_default(&self) -> bool {
        DaWrite::is_default(self)
    }
}

make_compound_impl! {
    AccountDiff < (), crate::DaError > u8 => AccountDiffTarget {
        balance: counter (CtrU64BySignedVarInt),
        snark: compound (SnarkAccountDiff),
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AccountDiffTarget {
    pub balance: u64,
    pub snark: SnarkAccountTarget,
}
