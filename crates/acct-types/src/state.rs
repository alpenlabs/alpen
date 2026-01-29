use strata_identifiers::Hash;

use crate::{
    AccountSerial, AccountTypeId, BitcoinAmount, RawAccountTypeId,
    ssz_generated::ssz::state::{
        AccountIntrinsicState, AcctStateSummary, EncodedAccountInnerState,
    },
};

impl EncodedAccountInnerState {
    pub fn raw_ty(&self) -> RawAccountTypeId {
        self.intrinsics.raw_ty()
    }

    /// Gets the type as a valid [`AccountTypeId`].
    pub fn ty(&self) -> AccountTypeId {
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

    pub fn typed_state_root(&self) -> &Hash {
        (&self.typed_state_root).into()
    }
}

impl AccountIntrinsicState {
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
    pub fn ty(&self) -> AccountTypeId {
        AccountTypeId::try_from(self.raw_ty()).expect("acct: invalid id")
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
    use proptest::prelude::*;
    use ssz::{Decode, Encode};
    use strata_codec::VARINT_MAX;
    use strata_test_utils_ssz::ssz_proptest;

    use super::*;

    mod account_intrinsic_state {
        use super::*;

        ssz_proptest!(
            AccountIntrinsicState,
            (any::<u16>(), 0..=VARINT_MAX, any::<u64>()).prop_map(|(raw_ty, serial, sats)| {
                AccountIntrinsicState {
                    raw_ty,
                    serial: AccountSerial::new(serial).expect("serial is within varint bounds"),
                    balance: BitcoinAmount::from_sat(sats),
                }
            })
        );

        #[test]
        fn test_zero_ssz() {
            let state = AccountIntrinsicState::new_empty(AccountSerial::zero());
            let encoded = state.as_ssz_bytes();
            let decoded = AccountIntrinsicState::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(state.raw_ty(), decoded.raw_ty());
            assert_eq!(state.serial(), decoded.serial());
            assert_eq!(state.balance(), decoded.balance());
        }
    }

    mod encoded_account_inner_state {
        use super::*;

        ssz_proptest!(
            EncodedAccountInnerState,
            (
                any::<u16>(),
                0..=VARINT_MAX,
                any::<u64>(),
                prop::collection::vec(any::<u8>(), 0..100)
            )
                .prop_map(|(raw_ty, serial, sats, encoded)| {
                    EncodedAccountInnerState {
                        intrinsics: AccountIntrinsicState {
                            raw_ty,
                            serial: AccountSerial::new(serial)
                                .expect("serial is within varint bounds"),
                            balance: BitcoinAmount::from_sat(sats),
                        },
                        encoded_state: encoded.into(),
                    }
                })
        );

        #[test]
        fn test_zero_ssz() {
            let state = EncodedAccountInnerState {
                intrinsics: AccountIntrinsicState::new_empty(
                    AccountSerial::new(0).expect("serial is within varint bounds"),
                ),
                encoded_state: vec![].into(),
            };
            let encoded = state.as_ssz_bytes();
            let decoded = EncodedAccountInnerState::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(state.raw_ty(), decoded.raw_ty());
            assert_eq!(state.serial(), decoded.serial());
            assert_eq!(state.balance(), decoded.balance());
            assert_eq!(state.encoded_state_buf(), decoded.encoded_state_buf());
        }
    }

    mod acct_state_summary {
        use super::*;

        ssz_proptest!(
            AcctStateSummary,
            (
                any::<u16>(),
                0..=VARINT_MAX,
                any::<u64>(),
                any::<[u8; 32]>()
            )
                .prop_map(|(raw_ty, serial, sats, root)| {
                    AcctStateSummary {
                        intrinsics: AccountIntrinsicState {
                            raw_ty,
                            serial: AccountSerial::new(serial)
                                .expect("serial is within varint bounds"),
                            balance: BitcoinAmount::from_sat(sats),
                        },
                        typed_state_root: root.into(),
                    }
                })
        );

        #[test]
        fn test_zero_ssz() {
            let state = AcctStateSummary {
                intrinsics: AccountIntrinsicState::new_empty(
                    AccountSerial::new(0).expect("serial is within varint bounds"),
                ),
                typed_state_root: [0u8; 32].into(),
            };
            let encoded = state.as_ssz_bytes();
            let decoded = AcctStateSummary::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(state.raw_ty(), decoded.raw_ty());
            assert_eq!(state.serial(), decoded.serial());
            assert_eq!(state.balance(), decoded.balance());
            assert_eq!(state.typed_state_root(), decoded.typed_state_root());
        }
    }
}
