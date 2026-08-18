//! Account diff types.

use strata_da_framework::counter_schemes::CtrU64BySignedVarInt;
use strata_da_framework::{DaCounter, DaWrite, make_compound_impl};
use strata_ol_da_common::DaError;

use super::snark::{SnarkAccountDiffV1, SnarkAccountTargetV1};

/// Per-account diff keyed by account type.
///
/// The account type is implied by pre-state; the snark field is only populated
/// for snark accounts.
#[derive(Debug)]
pub struct AccountDiffV1 {
    /// Balance counter diff (signed delta in satoshis).
    pub balance: DaCounter<CtrU64BySignedVarInt>,

    /// Snark state diff.
    pub snark: SnarkAccountDiffV1,
}

impl Default for AccountDiffV1 {
    fn default() -> Self {
        Self {
            balance: DaCounter::new_unchanged(),
            snark: SnarkAccountDiffV1::default(),
        }
    }
}

impl AccountDiffV1 {
    /// Creates a new account diff.
    pub fn new(balance: DaCounter<CtrU64BySignedVarInt>, snark: SnarkAccountDiffV1) -> Self {
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
    AccountDiffV1 < (), DaError > u8 => AccountDiffTargetV1 {
        balance: counter (CtrU64BySignedVarInt),
        snark: compound (SnarkAccountDiffV1),
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AccountDiffTargetV1 {
    pub balance: u64,
    pub snark: SnarkAccountTargetV1,
}
