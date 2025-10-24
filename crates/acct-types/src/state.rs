//! Account state types.

use crate::{
    errors::{AcctError, AcctResult},
    id::{AccountTypeId, RawAccountTypeId},
    mmr::Hash,
};

type Root = Hash;

// Include SSZ type definitions from acct-ssz-types
include!("../../acct-ssz-types/src/state.rs");

// Business logic for AccountState
impl AccountState {
    pub fn raw_ty(&self) -> RawAccountTypeId {
        self.intrinsics.raw_ty()
    }

    /// Attempts to parse the type into a valid [`AccountTypeId`].
    pub fn ty(&self) -> AcctResult<AccountTypeId> {
        self.intrinsics.ty()
    }

    pub fn serial(&self) -> AccountSerial {
        self.intrinsics.serial()
    }

    pub fn balance(&self) -> BitcoinAmount {
        self.intrinsics.balance()
    }

    pub fn encoded_state_buf(&self) -> &[u8] {
        &self.encoded_state
    }

    /// Attempts to decode the account state as a concrete account type.
    ///
    /// This MUST match, returns error otherwise.
    pub fn decode_as_type<T: AccountTypeState>(&self) -> AcctResult<T> {
        let real_ty = self.ty()?;
        if T::ID != real_ty {
            return Err(AcctError::MismatchedType(real_ty, T::ID));
        }

        // TODO
        unimplemented!()
    }
}

// Business logic for AcctStateSummary
impl AcctStateSummary {
    pub fn raw_ty(&self) -> RawAccountTypeId {
        AccountTypeId::try_from(self.serial.0 as u16)
            .map(|ty| ty as RawAccountTypeId)
            .unwrap_or(0)
    }

    pub fn serial(&self) -> AccountSerial {
        self.serial
    }

    pub fn balance(&self) -> BitcoinAmount {
        self.balance
    }

    pub fn typed_state_root(&self) -> &Root {
        &self.state_root
    }
}

// Business logic for IntrinsicAccountState
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

    /// Attempts to parse the type into a valid [`AccountTypeId`].
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

#[cfg(test)]
mod tests {
    use ssz::{Decode, Encode};

    use super::*;
    use crate::BitcoinAmount;

    #[test]
    fn test_intrinsic_account_state_ssz_roundtrip() {
        let state = IntrinsicAccountState::new(
            AccountTypeId::Snark,
            AccountSerial(10),
            BitcoinAmount::from_sat(5000),
        );
        let encoded = state.as_ssz_bytes();
        let decoded = IntrinsicAccountState::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(state, decoded);
    }

    #[test]
    fn test_account_state_ssz_roundtrip() {
        let state = AccountState {
            intrinsics: IntrinsicAccountState::new(
                AccountTypeId::Empty,
                AccountSerial(1),
                BitcoinAmount::from_sat(1000),
            ),
            encoded_state: vec![1, 2, 3, 4].into(),
        };
        let encoded = state.as_ssz_bytes();
        let decoded = AccountState::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(state.intrinsics, decoded.intrinsics);
        assert_eq!(state.encoded_state_buf(), decoded.encoded_state_buf());
    }

    #[test]
    fn test_acct_state_summary_ssz_roundtrip() {
        let summary = AcctStateSummary {
            serial: AccountSerial(42),
            balance: BitcoinAmount::from_sat(9999),
            state_root: [0xAB; 32],
        };
        let encoded = summary.as_ssz_bytes();
        let decoded = AcctStateSummary::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(summary, decoded);
    }

    #[test]
    fn test_intrinsic_account_state_with_new_balance() {
        let state = IntrinsicAccountState::new_empty(AccountSerial(5));
        let new_state = state.with_new_balance(BitcoinAmount::from_sat(1234));
        assert_eq!(new_state.balance(), BitcoinAmount::from_sat(1234));
        assert_eq!(new_state.serial(), AccountSerial(5));
    }
}
