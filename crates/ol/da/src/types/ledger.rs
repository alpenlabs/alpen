//! Ledger diff types.

use strata_acct_types::{AccountId, BitcoinAmount, Hash};
use strata_codec::{Codec, CodecError, Decoder, Encoder};
use strata_identifiers::{AccountSerial, AccountTypeId};

use super::{
    MAX_VK_BYTES,
    account::AccountDiff,
    encoding::{U16LenBytes, U16LenList},
};

/// Diff of ledger state (new accounts + account diffs).
#[derive(Debug, Codec)]
pub struct LedgerDiff {
    /// New accounts created during the epoch.
    pub new_accounts: U16LenList<NewAccountEntry>,

    /// Per-account diffs for touched accounts.
    pub account_diffs: U16LenList<AccountDiffEntry>,
}

impl Default for LedgerDiff {
    fn default() -> Self {
        Self {
            new_accounts: U16LenList::new(Vec::new()),
            account_diffs: U16LenList::new(Vec::new()),
        }
    }
}

impl LedgerDiff {
    /// Creates a new [`LedgerDiff`] from a list of new accounts and account diffs.
    pub fn new(
        new_accounts: U16LenList<NewAccountEntry>,
        account_diffs: U16LenList<AccountDiffEntry>,
    ) -> Self {
        Self {
            new_accounts,
            account_diffs,
        }
    }

    /// Returns true when no ledger changes are present.
    pub fn is_empty(&self) -> bool {
        self.new_accounts.entries().is_empty() && self.account_diffs.entries().is_empty()
    }
}

/// New account initialization entry.
#[derive(Clone, Debug, Eq, PartialEq, Codec)]
pub struct NewAccountEntry {
    /// Assigned account serial number.
    pub serial: AccountSerial,

    /// Account identifier.
    pub account_id: AccountId,

    /// Initial account data.
    pub init: AccountInit,
}

impl NewAccountEntry {
    /// Creates a new [`NewAccountEntry`] from a serial, account ID, and initial data.
    pub fn new(serial: AccountSerial, account_id: AccountId, init: AccountInit) -> Self {
        Self {
            serial,
            account_id,
            init,
        }
    }
}

/// Account initialization data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountInit {
    /// Initial balance for the account.
    pub balance: BitcoinAmount,

    /// Initial type-specific state.
    pub type_state: AccountTypeInit,
}

impl AccountInit {
    /// Creates a new [`AccountInit`] from a balance and type-specific state.
    pub fn new(balance: BitcoinAmount, type_state: AccountTypeInit) -> Self {
        Self {
            balance,
            type_state,
        }
    }

    /// Returns the account type ID.
    pub fn type_id(&self) -> AccountTypeId {
        match self.type_state {
            AccountTypeInit::Empty => AccountTypeId::Empty,
            AccountTypeInit::Snark(_) => AccountTypeId::Snark,
        }
    }
}

impl Codec for AccountInit {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.balance.encode(enc)?;
        let type_id = match self.type_state {
            AccountTypeInit::Empty => 0u8,
            AccountTypeInit::Snark(_) => 1u8,
        };
        type_id.encode(enc)?;
        match &self.type_state {
            AccountTypeInit::Empty => Ok(()),
            AccountTypeInit::Snark(init) => init.encode(enc),
        }
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let balance = BitcoinAmount::decode(dec)?;
        let raw_type_id = u8::decode(dec)?;
        let type_state = match raw_type_id {
            0 => AccountTypeInit::Empty,
            1 => AccountTypeInit::Snark(SnarkAccountInit::decode(dec)?),
            _ => return Err(CodecError::InvalidVariant("account_type_id")),
        };
        Ok(Self {
            balance,
            type_state,
        })
    }
}

/// Type-specific initial state for new accounts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AccountTypeInit {
    /// Empty account with no type state.
    Empty,

    /// Snark account with initial snark state.
    Snark(SnarkAccountInit),
}

/// Snark account initialization data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnarkAccountInit {
    /// Initial inner state root.
    pub initial_state_root: Hash,

    /// Update verification key bytes.
    pub update_vk: U16LenBytes,
}

impl SnarkAccountInit {
    /// Creates a new [`SnarkAccountInit`] from a initial state root and update verification key.
    pub fn new(initial_state_root: Hash, update_vk: Vec<u8>) -> Self {
        Self {
            initial_state_root,
            update_vk: U16LenBytes::new(update_vk),
        }
    }
}

impl Codec for SnarkAccountInit {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.initial_state_root.encode(enc)?;
        self.update_vk.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let initial_state_root = Hash::decode(dec)?;
        let update_vk = U16LenBytes::decode(dec)?;
        if update_vk.as_slice().len() > MAX_VK_BYTES {
            return Err(CodecError::OverflowContainer);
        }
        Ok(Self {
            initial_state_root,
            update_vk,
        })
    }
}

/// Per-account diff entry keyed by account serial.
#[derive(Debug, Codec)]
pub struct AccountDiffEntry {
    /// Account serial number.
    pub account_serial: AccountSerial,

    /// Per-account diff.
    pub diff: AccountDiff,
}

impl AccountDiffEntry {
    /// Creates a new [`AccountDiffEntry`] from a serial and diff.
    pub fn new(account_serial: AccountSerial, diff: AccountDiff) -> Self {
        Self {
            account_serial,
            diff,
        }
    }
}
