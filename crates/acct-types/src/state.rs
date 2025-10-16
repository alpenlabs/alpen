use ssz_derive::{Decode, Encode};
use ssz_types::VariableList;
use tree_hash_derive::TreeHash;

use crate::{
    amount::BitcoinAmount,
    errors::{AcctError, AcctResult},
    id::{AccountSerial, AccountTypeId, RawAccountTypeId},
    mmr::Hash,
};

type Root = Hash;

/// Variable-length byte list for account encoded state (max 64 KiB)
type AccountEncodedState = VariableList<u8, 65536>;

/// Account state.
// TODO builder
#[derive(Clone, Debug, Encode, Decode, TreeHash)]
pub struct AccountState {
    intrinsics: IntrinsicAccountState,
    encoded_state: AccountEncodedState,
}

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

    // should this even be exposed?
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

/// SSZ summary *structure*, not equivalent encoding.  It's an SSZ thing.
#[derive(Clone, Debug, Encode, Decode, TreeHash)]
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
#[derive(Copy, Clone, Debug, Eq, PartialEq, Encode, Decode, TreeHash)]
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
    use ssz_types::VariableList;
    use tree_hash::TreeHash;

    use super::*;

    #[test]
    fn test_intrinsic_account_state_ssz_roundtrip() {
        let state = IntrinsicAccountState::new(
            AccountTypeId::Snark,
            AccountSerial::new(42),
            BitcoinAmount::new(1000),
        );

        let encoded = state.as_ssz_bytes();
        let decoded = IntrinsicAccountState::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(state, decoded);
    }

    #[test]
    fn test_intrinsic_account_state_tree_hash() {
        let state1 = IntrinsicAccountState::new(
            AccountTypeId::Empty,
            AccountSerial::new(1),
            BitcoinAmount::new(100),
        );

        let state2 = IntrinsicAccountState::new(
            AccountTypeId::Empty,
            AccountSerial::new(1),
            BitcoinAmount::new(100),
        );

        // Same state should produce same hash
        assert_eq!(state1.tree_hash_root(), state2.tree_hash_root());
    }

    #[test]
    fn test_account_state_ssz_roundtrip() {
        let intrinsics = IntrinsicAccountState::new(
            AccountTypeId::Snark,
            AccountSerial::new(5),
            BitcoinAmount::new(5000),
        );

        let encoded_state_data = vec![1, 2, 3, 4, 5];
        let state = AccountState {
            intrinsics,
            encoded_state: VariableList::from(encoded_state_data.clone()),
        };

        let encoded = state.as_ssz_bytes();
        let decoded = AccountState::from_ssz_bytes(&encoded).unwrap();

        assert_eq!(state.raw_ty(), decoded.raw_ty());
        assert_eq!(state.serial(), decoded.serial());
        assert_eq!(state.balance(), decoded.balance());
        assert_eq!(state.encoded_state_buf(), decoded.encoded_state_buf());
    }

    #[test]
    fn test_account_state_with_empty_encoded_state() {
        let intrinsics = IntrinsicAccountState::new_empty(AccountSerial::new(10));
        let state = AccountState {
            intrinsics,
            encoded_state: VariableList::from(vec![]),
        };

        let encoded = state.as_ssz_bytes();
        let decoded = AccountState::from_ssz_bytes(&encoded).unwrap();

        assert!(decoded.encoded_state_buf().is_empty());
        assert_eq!(decoded.balance(), BitcoinAmount::zero());
    }

    #[test]
    fn test_acct_state_summary_ssz_roundtrip() {
        let intrinsics = IntrinsicAccountState::new(
            AccountTypeId::Snark,
            AccountSerial::new(100),
            BitcoinAmount::new(10000),
        );

        let summary = AcctStateSummary {
            intrinsics,
            typed_state_root: [55u8; 32],
        };

        let encoded = summary.as_ssz_bytes();
        let decoded = AcctStateSummary::from_ssz_bytes(&encoded).unwrap();

        assert_eq!(summary.raw_ty(), decoded.raw_ty());
        assert_eq!(summary.serial(), decoded.serial());
        assert_eq!(summary.balance(), decoded.balance());
        assert_eq!(summary.typed_state_root(), decoded.typed_state_root());
    }

    #[test]
    fn test_account_state_large_encoded_state() {
        let intrinsics = IntrinsicAccountState::new(
            AccountTypeId::Empty,
            AccountSerial::new(1),
            BitcoinAmount::new(1),
        );

        // Test with large encoded state (1 KiB)
        let large_data = vec![0xABu8; 1024];
        let state = AccountState {
            intrinsics,
            encoded_state: VariableList::from(large_data.clone()),
        };

        let encoded = state.as_ssz_bytes();
        let decoded = AccountState::from_ssz_bytes(&encoded).unwrap();

        assert_eq!(state.encoded_state_buf(), decoded.encoded_state_buf());
        assert_eq!(decoded.encoded_state_buf().len(), 1024);
    }
}
