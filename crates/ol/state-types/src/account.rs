use strata_acct_types::*;
use strata_codec::{Codec, CodecError, Decoder, Encoder};
use strata_identifiers::AccountSerial;
use strata_ledger_types::{AccountTypeState, *};

use crate::snark_account::NativeSnarkAccountState;

#[derive(Clone, Debug, Eq, PartialEq, Codec)]
pub struct NativeAccountState {
    serial: AccountSerial,
    balance: BitcoinAmount,
    state: NativeAccountTypeState,
}

impl NativeAccountState {
    /// Creates a new account state.
    pub fn new(
        serial: AccountSerial,
        balance: BitcoinAmount,
        state: NativeAccountTypeState,
    ) -> Self {
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

impl IAccountState for NativeAccountState {
    type SnarkAccountState = NativeSnarkAccountState;

    fn serial(&self) -> AccountSerial {
        self.serial
    }

    fn balance(&self) -> BitcoinAmount {
        self.balance
    }

    fn ty(&self) -> AccountTypeId {
        self.state.ty()
    }

    fn type_state(&self) -> AccountTypeStateRef<'_, Self> {
        match &self.state {
            NativeAccountTypeState::Empty => AccountTypeStateRef::Empty,
            NativeAccountTypeState::Snark(state) => AccountTypeStateRef::Snark(state),
        }
    }

    fn as_snark_account(&self) -> AcctResult<&Self::SnarkAccountState> {
        match &self.state {
            NativeAccountTypeState::Snark(state) => Ok(state),
            _ => Err(AcctError::MismatchedType(self.ty(), AccountTypeId::Snark)),
        }
    }
}

impl IAccountStateMut for NativeAccountState {
    type SnarkAccountStateMut = NativeSnarkAccountState;

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
            NativeAccountTypeState::Snark(state) => Ok(state),
            _ => Err(AcctError::MismatchedType(ty, AccountTypeId::Snark)),
        }
    }
}

impl IAccountStateConstructible for NativeAccountState {
    fn new_with_serial(new_acct_data: NewAccountData<Self>, serial: AccountSerial) -> Self {
        Self::new(
            serial,
            new_acct_data.initial_balance(),
            NativeAccountTypeState::from_generic(new_acct_data.into_type_state()),
        )
    }
}

/// Internal impl of account state types.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NativeAccountTypeState {
    /// An empty/inert account entry that holds a balance but nothing else.
    ///
    /// Usable for testing/internal purposes.
    Empty,

    /// A snark account.
    Snark(NativeSnarkAccountState),
}

impl NativeAccountTypeState {
    /// Returns the account type ID for this state.
    pub fn ty(&self) -> AccountTypeId {
        match self {
            Self::Empty => AccountTypeId::Empty,
            Self::Snark(_) => AccountTypeId::Snark,
        }
    }

    /// Converts from the generic wrapper.
    pub fn from_generic(ts: AccountTypeState<NativeAccountState>) -> Self {
        match ts {
            AccountTypeState::Empty => Self::Empty,
            AccountTypeState::Snark(s) => Self::Snark(s),
        }
    }

    /// Converts into the generic wrapper.
    pub fn into_generic(self) -> AccountTypeState<NativeAccountState> {
        match self {
            NativeAccountTypeState::Empty => AccountTypeState::Empty,
            NativeAccountTypeState::Snark(s) => AccountTypeState::Snark(s),
        }
    }
}

impl Codec for NativeAccountTypeState {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        // Encode the variant discriminant
        match self {
            Self::Empty => {
                0u8.encode(enc)?;
            }
            Self::Snark(state) => {
                1u8.encode(enc)?;
                state.encode(enc)?;
            }
        }
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let variant = u8::decode(dec)?;
        match variant {
            0 => Ok(Self::Empty),
            1 => Ok(Self::Snark(NativeSnarkAccountState::decode(dec)?)),
            _ => Err(CodecError::InvalidVariant("NativeAccountTypeState")),
        }
    }
}
