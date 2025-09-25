use std::fmt;

use crate::{
    amount::BitcoinAmount,
    errors::{AcctError, AcctResult},
    id::{AccountId, AccountSerial, AccountTypeId, RawAccountTypeId},
    mmr::Hash,
};

type Root = Hash;

/// Account state.
// TODO SSZ
// TODO builder
#[derive(Clone, Debug)]
pub struct AccountState {
    intrinsics: IntrinsicAccountState,
    encoded_state: Vec<u8>,
}

impl AccountState {
    pub fn raw_ty(&self) -> RawAccountTypeId {
        self.intrinsics.raw_ty()
    }

    /// Attempts to parse the type into a valid [`AcctTypeId`].
    pub fn ty(&self) -> AcctResult<AccountTypeId> {
        self.intrinsics.ty()
    }

    pub fn serial(&self) -> AccountSerial {
        self.intrinsics.serial()
    }

    pub fn balance(&self) -> BitcoinAmount {
        self.intrinsics.balance()
    }

    // should this even be exposed?
    pub fn encoded_state_buf(&self) -> &[u8] {
        &self.encoded_state
    }

    /// Attempts to decode the account state as a concrete account type.
    ///
    /// This MUST match, returns error otherwise.
    pub fn decode_as_type<T: AccountTypeState>(&self) -> AcctResult<T> {
        let dec_ty = T::ID;
        let real_ty = self.ty()?;
        if T::ID != self.ty()? {
            return Err(AcctError::MismatchedType(real_ty, T::ID));
        }

        // TODO
        unimplemented!()
    }
}

/// SSZ summary *structure*, not equivalent encoding.  It's an SSZ thing.
// TODO SSZ
#[derive(Clone, Debug)]
pub struct AcctStateSummary {
    intrinsics: IntrinsicAccountState,
    typed_state_root: Root,
}

impl AcctStateSummary {
    pub fn raw_ty(&self) -> RawAccountTypeId {
        self.intrinsics.raw_ty()
    }

    pub fn serial(&self) -> AccountSerial {
        self.intrinsics.serial()
    }

    pub fn balance(&self) -> BitcoinAmount {
        self.intrinsics.balance()
    }

    pub fn typed_state_root(&self) -> &Root {
        &self.typed_state_root
    }
}

/// Intrinsic account fields.
// TODO SSZ
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct IntrinsicAccountState {
    // immutable fields, these MUST NOT change
    /// Account type, which determines how we interact with it.
    raw_ty: RawAccountTypeId,

    /// Account serial number.
    serial: AccountSerial,

    // mutable fields, which MAY change
    /// Native asset (satoshi) balance.
    balance: BitcoinAmount,
}

impl IntrinsicAccountState {
    /// Constructs a new raw instance.
    fn new_unchecked(
        raw_ty: RawAccountTypeId,
        serial: AccountSerial,
        balance: BitcoinAmount,
    ) -> Self {
        Self {
            raw_ty,
            serial,
            balance,
        }
    }

    /// Creates a new account using a real type ID.
    pub fn new(ty: AccountTypeId, serial: AccountSerial, balance: BitcoinAmount) -> Self {
        Self::new_unchecked(ty as RawAccountTypeId, serial, balance)
    }

    /// Creates a new empty account with no balance.
    pub fn new_empty(serial: AccountSerial) -> Self {
        Self::new(AccountTypeId::Empty, serial, 0.into())
    }

    pub fn raw_ty(&self) -> RawAccountTypeId {
        self.raw_ty
    }

    /// Attempts to parse the type into a valid [`AcctTypeId`].
    pub fn ty(&self) -> AcctResult<AccountTypeId> {
        AccountTypeId::try_from(self.raw_ty()).map_err(AcctError::InvalidAcctTypeId)
    }

    pub fn serial(&self) -> AccountSerial {
        self.serial
    }

    pub fn balance(&self) -> BitcoinAmount {
        self.balance
    }

    /// Constructs a new instance with an updated balance.
    pub fn with_new_balance(&self, bal: BitcoinAmount) -> Self {
        Self {
            balance: bal,
            ..*self
        }
    }
}

/// Helper trait for making account types.
pub trait AccountTypeState {
    /// Account type ID.
    const ID: AccountTypeId;

    // TODO decoding
}
