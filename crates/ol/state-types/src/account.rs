use strata_acct_types::*;
use strata_identifiers::AccountSerial;
use strata_ledger_types::{AccountTypeState, *};

use crate::ssz_generated::ssz::state::{OLAccountState, OLAccountTypeState, OLSnarkAccountState};

impl OLAccountState {
    /// Creates a new account state.
    pub fn new(serial: AccountSerial, balance: BitcoinAmount, state: OLAccountTypeState) -> Self {
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

impl IAccountState for OLAccountState {
    type SnarkAccountState = OLSnarkAccountState;

    fn serial(&self) -> AccountSerial {
        self.serial
    }

    fn balance(&self) -> BitcoinAmount {
        self.balance
    }

    fn ty(&self) -> AccountTypeId {
        match &self.state {
            OLAccountTypeState::Empty => AccountTypeId::Empty,
            OLAccountTypeState::Snark(_) => AccountTypeId::Snark,
        }
    }

    fn type_state(&self) -> AccountTypeStateRef<'_, Self> {
        match &self.state {
            OLAccountTypeState::Empty => AccountTypeStateRef::Empty,
            OLAccountTypeState::Snark(state) => AccountTypeStateRef::Snark(state),
        }
    }

    fn as_snark_account(&self) -> AcctResult<&Self::SnarkAccountState> {
        match &self.state {
            OLAccountTypeState::Snark(state) => Ok(state),
            _ => Err(AcctError::MismatchedType(self.ty(), AccountTypeId::Snark)),
        }
    }
}

impl IAccountStateMut for OLAccountState {
    type SnarkAccountStateMut = OLSnarkAccountState;

    fn add_balance(&mut self, coin: Coin) {
        self.balance = self
            .balance
            .checked_add(coin.amt())
            .expect("ledger: overflow balance");
        coin.safely_consume_unchecked();
    }

    fn take_balance(&mut self, amt: BitcoinAmount) -> AcctResult<Coin> {
        self.balance = self
            .balance
            .checked_sub(amt)
            .expect("ledger: underflow balance");
        Ok(Coin::new_unchecked(amt))
    }

    fn as_snark_account_mut(&mut self) -> AcctResult<&mut Self::SnarkAccountStateMut> {
        let ty = self.ty();
        match &mut self.state {
            OLAccountTypeState::Snark(state) => Ok(state),
            _ => Err(AcctError::MismatchedType(ty, AccountTypeId::Snark)),
        }
    }
}

impl IAccountStateConstructible for OLAccountState {
    fn new_with_serial(new_acct_data: NewAccountData<Self>, serial: AccountSerial) -> Self {
        Self::new(
            serial,
            new_acct_data.initial_balance(),
            OLAccountTypeState::from_generic(new_acct_data.into_type_state()),
        )
    }
}

impl OLAccountTypeState {
    /// Returns the account type ID for this state.
    pub fn ty(&self) -> AccountTypeId {
        match self {
            OLAccountTypeState::Empty => AccountTypeId::Empty,
            OLAccountTypeState::Snark(_) => AccountTypeId::Snark,
        }
    }

    /// Converts from the generic wrapper.
    pub fn from_generic(ts: AccountTypeState<OLAccountState>) -> Self {
        match ts {
            AccountTypeState::Empty => OLAccountTypeState::Empty,
            AccountTypeState::Snark(s) => OLAccountTypeState::Snark(s),
        }
    }

    /// Converts into the generic wrapper.
    pub fn into_generic(self) -> AccountTypeState<OLAccountState> {
        match self {
            OLAccountTypeState::Empty => AccountTypeState::Empty,
            OLAccountTypeState::Snark(s) => AccountTypeState::Snark(s),
        }
    }
}

#[cfg(test)]
mod tests {
    use strata_test_utils_ssz::ssz_proptest;

    use super::*;
    use crate::test_utils::{
        ol_account_state_strategy, ol_account_type_state_strategy, ol_snark_account_state_strategy,
    };

    mod ol_account_state {
        use super::*;
        ssz_proptest!(OLAccountState, ol_account_state_strategy());
    }

    mod ol_account_type_state {
        use super::*;
        ssz_proptest!(OLAccountTypeState, ol_account_type_state_strategy());
    }

    mod ol_snark_account_state {
        use super::*;
        ssz_proptest!(OLSnarkAccountState, ol_snark_account_state_strategy());
    }
}
