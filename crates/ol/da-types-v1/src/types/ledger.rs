//! Ledger diff types.

use strata_acct_types::{AccountId, BitcoinAmount, Hash};
use strata_codec::{Codec, CodecError, Decoder, Encoder};
use strata_identifiers::{AccountSerial, AccountTypeId};
use strata_ol_da_common::{U16LenBytes, U16LenList};

use super::MAX_VK_BYTES;
use super::account::AccountDiffV1;

/// Diff of ledger state (new accounts + account diffs).
#[derive(Debug, Codec)]
pub struct LedgerDiffV1 {
    /// New accounts created during the epoch.
    pub new_accounts: U16LenList<NewAccountEntryV1>,

    /// Per-account diffs for touched accounts.
    pub account_diffs: U16LenList<AccountDiffEntryV1>,
}

impl Default for LedgerDiffV1 {
    fn default() -> Self {
        Self {
            new_accounts: U16LenList::new(Vec::new()),
            account_diffs: U16LenList::new(Vec::new()),
        }
    }
}

impl LedgerDiffV1 {
    /// Creates a new [`LedgerDiffV1`] from a list of new accounts and account diffs.
    pub fn new(
        new_accounts: U16LenList<NewAccountEntryV1>,
        account_diffs: U16LenList<AccountDiffEntryV1>,
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
pub struct NewAccountEntryV1 {
    /// Account identifier.
    pub account_id: AccountId,

    /// Initial account data.
    pub init: AccountInitV1,
}

impl NewAccountEntryV1 {
    /// Creates a new [`NewAccountEntryV1`] from an account ID and initial data.
    ///
    /// The account serial is inferred from context by applying entries in order.
    pub fn new(account_id: AccountId, init: AccountInitV1) -> Self {
        Self { account_id, init }
    }
}

/// Account initialization data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountInitV1 {
    /// Initial balance for the account.
    pub balance: BitcoinAmount,

    /// Initial type-specific state.
    pub type_state: AccountTypeInitV1,
}

impl AccountInitV1 {
    /// Creates a new [`AccountInitV1`] from a balance and type-specific state.
    pub fn new(balance: BitcoinAmount, type_state: AccountTypeInitV1) -> Self {
        Self {
            balance,
            type_state,
        }
    }

    /// Returns the account type ID.
    pub fn type_id(&self) -> AccountTypeId {
        match self.type_state {
            AccountTypeInitV1::Empty => AccountTypeId::Empty,
            AccountTypeInitV1::Snark(_) => AccountTypeId::Snark,
        }
    }
}

impl Codec for AccountInitV1 {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.balance.encode(enc)?;
        let type_id = match self.type_state {
            AccountTypeInitV1::Empty => 0u8,
            AccountTypeInitV1::Snark(_) => 1u8,
        };
        type_id.encode(enc)?;
        match &self.type_state {
            AccountTypeInitV1::Empty => Ok(()),
            AccountTypeInitV1::Snark(init) => init.encode(enc),
        }
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let balance = BitcoinAmount::decode(dec)?;
        let raw_type_id = u8::decode(dec)?;
        let type_state = match raw_type_id {
            0 => AccountTypeInitV1::Empty,
            1 => AccountTypeInitV1::Snark(SnarkAccountInitV1::decode(dec)?),
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
pub enum AccountTypeInitV1 {
    /// Empty account with no type state.
    Empty,

    /// Snark account with initial snark state.
    Snark(SnarkAccountInitV1),
}

/// Snark account initialization data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnarkAccountInitV1 {
    /// Initial inner state root.
    pub initial_state_root: Hash,

    /// Update verification key bytes (u16 length prefix per SPS-ol-da-structure).
    pub update_vk: U16LenBytes,
}

impl SnarkAccountInitV1 {
    /// Creates a new [`SnarkAccountInitV1`] from a initial state root and update verification key.
    pub fn new(initial_state_root: Hash, update_vk: Vec<u8>) -> Self {
        Self {
            initial_state_root,
            update_vk: U16LenBytes::new(update_vk),
        }
    }
}

impl Codec for SnarkAccountInitV1 {
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
pub struct AccountDiffEntryV1 {
    /// Account serial number.
    pub account_serial: AccountSerial,

    /// Per-account diff.
    pub diff: AccountDiffV1,
}

impl AccountDiffEntryV1 {
    /// Creates a new [`AccountDiffEntryV1`] from a serial and diff.
    pub fn new(account_serial: AccountSerial, diff: AccountDiffV1) -> Self {
        Self {
            account_serial,
            diff,
        }
    }
}
