use std::fmt;

use crate::{
    errors::{AcctError, AcctResult},
    id::{AcctId, AcctTypeId},
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
    pub fn ty(&self) -> u16 {
        self.intrinsics.ty()
    }

    pub fn serial(&self) -> AcctId {
        self.intrinsics.serial()
    }

    pub fn balance(&self) -> u64 {
        self.intrinsics.balance()
    }

    pub fn encoded_state_buf(&self) -> &[u8] {
        &self.encoded_state
    }

    pub fn decode_as_type<T: AcctTypeState>(&self) -> AcctResult<T> {
        let dec_ty = T::ID;
        if T::ID as u16 != self.ty() {
            let id = AcctTypeId::from(self.ty());
            return Err(AcctError::MismatchedType(AcctTypeId::Empty, T::ID));
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
    pub fn ty(&self) -> u16 {
        self.intrinsics.ty()
    }

    pub fn serial(&self) -> AcctId {
        self.intrinsics.serial()
    }

    pub fn balance(&self) -> u64 {
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
    ty: u16,

    /// Account serial number.
    serial: AcctId,

    // mutable fields, which MAY change
    /// Native asset (satoshi) balance.
    balance: u64,
}

impl IntrinsicAcctState {
    pub fn new(ty: u16, serial: AcctId, balance: u64) -> Self {
        Self {
            ty,
            serial,
            balance,
        }
    }

    pub fn ty(&self) -> u16 {
        self.ty
    }

    pub fn serial(&self) -> AcctId {
        self.serial
    }

    pub fn balance(&self) -> u64 {
        self.balance
    }

    /// Constructs a new instance with an updated balance.
    pub fn with_new_balance(&self, bal: u64) -> Self {
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
