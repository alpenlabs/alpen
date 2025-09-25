use std::fmt;

use crate::{
    amount::BitcoinAmount,
    errors::{AcctError, AcctResult},
    id::{AcctId, AcctSerial, AcctTypeId, RawAcctTypeId},
    mmr::Hash,
};

type Root = Hash;

/// Account state.
// TODO SSZ
// TODO builder
#[derive(Clone, Debug)]
pub struct AcctState {
    intrinsics: IntrinsicAcctState,
    encoded_state: Vec<u8>,
}

impl AcctState {
    pub fn raw_ty(&self) -> RawAcctTypeId {
        self.intrinsics.raw_ty()
    }

    /// Attempts to parse the type into a valid [`AcctTypeId`].
    pub fn ty(&self) -> AcctResult<AcctTypeId> {
        self.intrinsics.ty()
    }

    pub fn serial(&self) -> AcctSerial {
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
    pub fn decode_as_type<T: AcctTypeState>(&self) -> AcctResult<T> {
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
    intrinsics: IntrinsicAcctState,
    typed_state_root: Root,
}

impl AcctStateSummary {
    pub fn raw_ty(&self) -> RawAcctTypeId {
        self.intrinsics.raw_ty()
    }

    pub fn serial(&self) -> AcctSerial {
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
pub struct IntrinsicAcctState {
    // immutable fields, these MUST NOT change
    /// Account type, which determines how we interact with it.
    raw_ty: RawAcctTypeId,

    /// Account serial number.
    serial: AcctSerial,

    // mutable fields, which MAY change
    /// Native asset (satoshi) balance.
    balance: BitcoinAmount,
}

impl IntrinsicAcctState {
    /// Constructs a new raw instance.
    fn new_unchecked(raw_ty: RawAcctTypeId, serial: AcctSerial, balance: BitcoinAmount) -> Self {
        Self {
            raw_ty,
            serial,
            balance,
        }
    }

    /// Creates a new account using a real type ID.
    pub fn new(ty: AcctTypeId, serial: AcctSerial, balance: BitcoinAmount) -> Self {
        Self::new_unchecked(ty as RawAcctTypeId, serial, balance)
    }

    /// Creates a new empty account with no balance.
    pub fn new_empty(serial: AcctSerial) -> Self {
        Self::new(AcctTypeId::Empty, serial, 0.into())
    }

    pub fn raw_ty(&self) -> RawAcctTypeId {
        self.raw_ty
    }

    /// Attempts to parse the type into a valid [`AcctTypeId`].
    pub fn ty(&self) -> AcctResult<AcctTypeId> {
        AcctTypeId::try_from(self.raw_ty()).map_err(AcctError::InvalidAcctTypeId)
    }

    pub fn serial(&self) -> AcctSerial {
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
pub trait AcctTypeState {
    /// Account type ID.
    const ID: AcctTypeId;

    // TODO decoding
}
