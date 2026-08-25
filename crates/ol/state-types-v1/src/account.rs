use strata_acct_types::*;
use strata_identifiers::AccountSerial;
use strata_ol_state_types::*;

use crate::ssz_generated::ssz::state::{
    OLAccountStateV1, OLAccountTypeStateV1, OLSnarkAccountStateV1,
};
use crate::write_batch::AccountStateWrite;

impl OLAccountStateV1 {
    /// Creates a new account state.
    pub fn new(serial: AccountSerial, balance: BitcoinAmount, state: OLAccountTypeStateV1) -> Self {
        Self {
            serial,
            balance,
            state,
        }
    }

    /// Returns the account serial.
    pub fn serial(&self) -> AccountSerial {
        self.serial
    }
}

impl IAccountState for OLAccountStateV1 {
    type SnarkAccountState = OLSnarkAccountStateV1;
    type Write = AccountStateWrite;

    fn new_with_serial(new_acct_data: NewAccountData, serial: AccountSerial) -> Self {
        let balance = new_acct_data.initial_balance();
        let type_state = match new_acct_data.into_type_state() {
            NewAccountTypeState::Empty => OLAccountTypeStateV1::Empty,
            NewAccountTypeState::Snark {
                update_vk,
                initial_state_root,
            } => OLAccountTypeStateV1::Snark(OLSnarkAccountStateV1::new_fresh(
                update_vk,
                initial_state_root,
            )),
        };
        Self::new(serial, balance, type_state)
    }

    fn apply_write(&mut self, write: Self::Write) -> StateResult<()> {
        if write.serial() != self.serial() {
            return Err(StateError::InapplicableAcctWrite {
                in_state: self.serial(),
                in_write: write.serial(),
            });
        }

        *self = write.into_state();
        Ok(())
    }

    fn serial(&self) -> AccountSerial {
        self.serial
    }

    fn balance(&self) -> BitcoinAmount {
        self.balance
    }

    fn ty(&self) -> AccountTypeId {
        match &self.state {
            OLAccountTypeStateV1::Empty => AccountTypeId::Empty,
            OLAccountTypeStateV1::Snark(_) => AccountTypeId::Snark,
        }
    }

    fn type_state(&self) -> AccountTypeStateRef<'_, Self> {
        match &self.state {
            OLAccountTypeStateV1::Empty => AccountTypeStateRef::Empty,
            OLAccountTypeStateV1::Snark(state) => AccountTypeStateRef::Snark(state),
        }
    }

    fn as_snark_account(&self) -> StateResult<&Self::SnarkAccountState> {
        match &self.state {
            OLAccountTypeStateV1::Snark(state) => Ok(state),
            _ => Err(StateError::MismatchedAcctType {
                got: self.ty(),
                expected: AccountTypeId::Snark,
            }),
        }
    }
}

impl IAccountStateMut for OLAccountStateV1 {
    type SnarkAccountStateMut = OLSnarkAccountStateV1;

    fn add_balance(&mut self, coin: Coin) {
        let balance_sats = self
            .balance
            .to_sat()
            .checked_add(coin.amt().to_sat())
            .expect("ledger: overflow balance");
        self.balance =
            BitcoinAmount::try_from(balance_sats).expect("ledger: balance exceeds money supply");
        coin.safely_consume_unchecked();
    }

    fn take_balance(&mut self, amt: BitcoinAmount) -> StateResult<Coin> {
        let balance_sats = self.balance.to_sat().checked_sub(amt.to_sat()).ok_or(
            StateError::InsufficientBalance {
                need: amt,
                have: self.balance,
            },
        )?;
        self.balance = BitcoinAmount::try_from(balance_sats)
            .expect("subtracting from a valid balance must remain valid");
        Ok(Coin::new_unchecked(amt))
    }

    fn as_snark_account_mut(&mut self) -> StateResult<&mut Self::SnarkAccountStateMut> {
        let ty = self.ty();
        match &mut self.state {
            OLAccountTypeStateV1::Snark(state) => Ok(state),
            _ => Err(StateError::MismatchedAcctType {
                got: ty,
                expected: AccountTypeId::Snark,
            }),
        }
    }
}

impl OLAccountTypeStateV1 {
    /// Returns the account type ID for this state.
    pub fn ty(&self) -> AccountTypeId {
        match self {
            OLAccountTypeStateV1::Empty => AccountTypeId::Empty,
            OLAccountTypeStateV1::Snark(_) => AccountTypeId::Snark,
        }
    }
}

#[cfg(test)]
mod tests {
    use strata_predicate::PredicateKey;
    use strata_test_utils_ssz::ssz_proptest;

    use super::*;
    use crate::test_utils::{
        ol_account_state_strategy, ol_account_type_state_strategy, ol_snark_account_state_strategy,
    };

    fn bitcoin_amount(sats: u64) -> BitcoinAmount {
        BitcoinAmount::try_from(sats).expect("amount must not exceed the Bitcoin money supply")
    }

    #[test]
    fn apply_write_replaces_state_with_matching_serial() {
        let serial = AccountSerial::from(7u32);
        let mut state =
            OLAccountStateV1::new(serial, bitcoin_amount(1_000), OLAccountTypeStateV1::Empty);
        let replacement = OLAccountStateV1::new(
            serial,
            bitcoin_amount(2_000),
            OLAccountTypeStateV1::Snark(OLSnarkAccountStateV1::new_fresh(
                PredicateKey::always_accept(),
                [1u8; 32].into(),
            )),
        );

        state
            .apply_write(AccountStateWrite::new(replacement))
            .unwrap();

        assert_eq!(state.balance(), bitcoin_amount(2_000));
        assert_eq!(state.ty(), AccountTypeId::Snark);
    }

    #[test]
    fn apply_write_rejects_mismatched_serial_without_modifying_state() {
        let state_serial = AccountSerial::from(7u32);
        let write_serial = AccountSerial::from(8u32);
        let mut state = OLAccountStateV1::new(
            state_serial,
            bitcoin_amount(1_000),
            OLAccountTypeStateV1::Empty,
        );
        let replacement = OLAccountStateV1::new(
            write_serial,
            bitcoin_amount(2_000),
            OLAccountTypeStateV1::Snark(OLSnarkAccountStateV1::new_fresh(
                PredicateKey::always_accept(),
                [1u8; 32].into(),
            )),
        );

        let error = state
            .apply_write(AccountStateWrite::new(replacement))
            .unwrap_err();

        match error {
            StateError::InapplicableAcctWrite { in_state, in_write } => {
                assert_eq!(in_state, state_serial);
                assert_eq!(in_write, write_serial);
            }
            error => panic!("unexpected error: {error}"),
        }
        assert_eq!(state.serial(), state_serial);
        assert_eq!(state.balance(), bitcoin_amount(1_000));
        assert_eq!(state.ty(), AccountTypeId::Empty);
    }

    mod ol_account_state {
        use super::*;
        ssz_proptest!(OLAccountStateV1, ol_account_state_strategy());
    }

    mod ol_account_type_state {
        use super::*;
        ssz_proptest!(OLAccountTypeStateV1, ol_account_type_state_strategy());
    }

    mod ol_snark_account_state {
        use super::*;
        ssz_proptest!(OLSnarkAccountStateV1, ol_snark_account_state_strategy());
    }
}
