use strata_acct_types::{
    AccountSerial, AccountTypeId, AcctResult, BitcoinAmount, RawAccountTypeId,
};
use strata_codec::{Codec, CodecError, Decoder, Encoder};
use strata_ledger_types::{AccountTypeState, Coin, IAccountState};

use crate::snark_account::NativeSnarkAccountState;

#[derive(Clone, Debug)]
pub struct AccountState {
    serial: AccountSerial,
    balance: BitcoinAmount,
    state: NativeAccountTypeState,
}

impl AccountState {
    pub(crate) fn new(
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

    pub(crate) fn serial(&self) -> AccountSerial {
        self.serial
    }
}

impl IAccountState for AccountState {
    type SnarkAccountState = NativeSnarkAccountState;

    fn serial(&self) -> AccountSerial {
        self.serial
    }

    fn balance(&self) -> BitcoinAmount {
        self.balance
    }

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

    // TODO should we remove this?  what value is it even still providing?
    fn raw_ty(&self) -> AcctResult<RawAccountTypeId> {
        Ok(self.state.ty() as RawAccountTypeId)
    }

    fn ty(&self) -> AcctResult<AccountTypeId> {
        Ok(self.state.ty())
    }

    fn get_type_state(&self) -> AcctResult<AccountTypeState<Self>> {
        Ok(self.state.clone().into_generic())
    }

    fn set_type_state(&mut self, state: AccountTypeState<Self>) -> AcctResult<()> {
        self.state = NativeAccountTypeState::from_generic(state);
        Ok(())
    }
}

// Codec implementation for AccountState
impl Codec for AccountState {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.serial.encode(enc)?;
        self.balance.to_sat().encode(enc)?;
        self.state.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let serial = AccountSerial::decode(dec)?;
        let balance = BitcoinAmount::from_sat(u64::decode(dec)?);
        let state = NativeAccountTypeState::decode(dec)?;
        Ok(Self {
            serial,
            balance,
            state,
        })
    }
}

/// Internal impl of account state types.
#[derive(Clone, Debug)]
pub(crate) enum NativeAccountTypeState {
    Empty,
    Snark(NativeSnarkAccountState),
}

impl NativeAccountTypeState {
    pub(crate) fn ty(&self) -> AccountTypeId {
        match self {
            Self::Empty => AccountTypeId::Empty,
            Self::Snark(_) => AccountTypeId::Snark,
        }
    }

    /// Converts from the generic wrapper.
    pub(crate) fn from_generic(ts: AccountTypeState<AccountState>) -> Self {
        match ts {
            AccountTypeState::Empty => Self::Empty,
            AccountTypeState::Snark(s) => Self::Snark(s),
        }
    }

    /// Converts into the generic wrapper.
    pub(crate) fn into_generic(self) -> AccountTypeState<AccountState> {
        match self {
            NativeAccountTypeState::Empty => AccountTypeState::Empty,
            NativeAccountTypeState::Snark(s) => AccountTypeState::Snark(s),
        }
    }
}

// Codec implementation for NativeAccountTypeState
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
            1 => {
                let state = NativeSnarkAccountState::decode(dec)?;
                Ok(Self::Snark(state))
            }
            _ => Err(CodecError::InvalidVariant("NativeAccountTypeState")),
        }
    }
}
