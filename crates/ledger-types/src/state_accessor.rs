use strata_acct_types::{AccountId, AccountSerial, AcctResult, BitcoinAmount};

use crate::{
    account::{AccountTypeState, IAccountState},
    global_state::IGlobalState,
    l1vs::IL1ViewState,
};

/// Opaque interface for manipulating the chainstate, for all of the parts
/// directly under the toplevel state.
///
/// This exists because we want to make this generic across the various
/// different contexts we'll be manipulating state.
pub trait StateAccessor {
    /// Type representing the global chainstate.
    type GlobalState: IGlobalState;

    /// Type representing L1 view state.
    type L1ViewState: IL1ViewState;

    /// Type representing a ledger account's state.
    type AccountState: IAccountState + Clone;

    /// Gets a ref to the global state.
    fn global(&self) -> &Self::GlobalState;

    /// Gets a mut ref to the global state.
    fn global_mut(&mut self) -> &mut Self::GlobalState;

    /// Gets a ref to the L1 view state.
    fn l1_view(&self) -> &Self::L1ViewState;

    /// Gets a mut ref to the L1 view state.
    fn set_l1_view(&mut self, l1v: Self::L1ViewState);

    /// Checks if an account exists.
    fn check_account_exists(&self, id: AccountId) -> AcctResult<bool>;

    /// Gets account id from serial.
    fn get_account_id_from_serial(&self, serial: AccountSerial) -> AcctResult<Option<AccountId>>;

    /// Gets a ref to an account, if it exists.
    fn get_account_state(&self, id: AccountId) -> AcctResult<Option<&Self::AccountState>>;

    /// Gets a mut ref to an account, if it exists.
    fn get_account_state_mut(
        &mut self,
        id: AccountId,
    ) -> AcctResult<Option<&mut Self::AccountState>>;

    /// Overwrites an existing account entry's state, if it exists.
    ///
    /// This refuses to create new accounts in order to avoid accidents like
    /// screwing up serials.
    fn update_account_state(&mut self, id: AccountId, state: Self::AccountState) -> AcctResult<()>;

    /// Creates a new account as some ID with some type state, if that ID
    /// doesn't exist, assigning it a fresh serial.  Returns the freshly created
    /// serial.
    fn create_new_account(
        &mut self,
        id: AccountId,
        state: AccountTypeState<Self::AccountState>,
    ) -> AcctResult<AccountSerial>;
}

/*
/// Type for interacting with the rest of the ledger.  There is implicit context
/// here that binds the interface to be operating from the perspective of the
/// current account (ie. attaching senders).
pub trait LedgerInterface {
    type LedgerError: Display;

    /// Current slot.
    fn cur_slot(&self) -> u64;

    /// Current epoch.
    fn cur_epoch(&self) -> u64;

    /// Account's balance.
    fn acct_balance(&self) -> BitcoinAmount;

    /// Checks if an account with given id exists.
    fn check_acct_exists(&self, acct_id: AccountId) -> bool;

    /// Send transfer to account.
    fn send_transfer(
        &self,
        acct_id: AccountId,
        amt: BitcoinAmount,
    ) -> Result<(), Self::LedgerError>;

    /// Send message to account.
    fn send_message(
        &self,
        acct_id: AccountId,
        msg: &[u8],
        amt: BitcoinAmount,
    ) -> Result<(), Self::LedgerError>;
}

/// Implementation of `LedgerInterface`
pub struct Ledger<S: StateAccessor> {
    state_accessor: S,
    account_state: AccountState,
}

impl<S: StateAccessor> Ledger<S> {
    pub fn new(state_accessor: S, account_state: AccountState) -> Self {
        Self {
            state_accessor,
            account_state,
        }
    }
}

impl LedgerInterface for Ledger {
    fn cur_slot(&self) -> u64 {
        self.state_accessor
    }
}
*/
