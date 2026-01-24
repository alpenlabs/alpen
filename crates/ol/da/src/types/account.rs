//! Account diff types.

use strata_acct_types::BitcoinAmount;
use strata_da_framework::{DaRegister, DaWrite, make_compound_impl};

use super::snark::{SnarkAccountDiff, SnarkAccountTarget};

/// Per-account diff keyed by account type.
///
/// The account type is implied by pre-state; the snark field is only populated
/// for snark accounts.
#[derive(Debug)]
pub struct AccountDiff {
    /// Balance register diff.
    pub balance: DaRegister<BitcoinAmount>,

    /// Snark state diff.
    pub snark: SnarkAccountDiff,
}

impl Default for AccountDiff {
    fn default() -> Self {
        Self {
            balance: DaRegister::new_unset(),
            snark: SnarkAccountDiff::default(),
        }
    }
}

impl AccountDiff {
    /// Creates a new account diff.
    pub fn new(balance: DaRegister<BitcoinAmount>, snark: SnarkAccountDiff) -> Self {
        Self { balance, snark }
    }

    /// Returns the balance diff, regardless of account type.
    pub fn balance(&self) -> &DaRegister<BitcoinAmount> {
        &self.balance
    }

    pub fn is_default(&self) -> bool {
        DaWrite::is_default(self)
    }
}

make_compound_impl! {
    AccountDiff u8 => AccountDiffTarget {
        balance: register (BitcoinAmount),
        snark: compound (SnarkAccountDiff),
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AccountDiffTarget {
    pub balance: BitcoinAmount,
    pub snark: SnarkAccountTarget,
}
