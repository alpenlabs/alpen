use strata_acct_types::{AccountId, AccountSerial, AcctResult};

use crate::{
    account::{AccountTypeState, IAccountState},
    global_state::IGlobalState,
};

/// Opaque interface for manipulating the chainstate, for all of the parts
/// directly under the toplevel state.
///
/// This exists because we want to make this generic across the various
/// different contexts we'll be manipulating state.
pub trait StateAccessor {
    /// Type representing the global chainstate.
    type GlobalState: IGlobalState;

    /// Type representing a ledger account's state.
    type AccountState: IAccountState;

    /// Gets a ref to the global state.
    fn global(&self) -> &Self::GlobalState;

    /// Gets a mut ref to the global state.
    fn globlal_mut(&mut self) -> &mut Self::GlobalState;

    /// Checks if an account exists.
    fn check_account_exists(&self, id: AccountId) -> AcctResult<bool>;

    /// Gets a ref to an account, if it exists.
    fn get_account_state(&self, id: AccountId) -> AcctResult<Option<&Self::AccountState>>;

    /// Gets a mut ref to an account, if it exists.
    fn get_account_state_mut(
        &mut self,
        id: AccountId,
    ) -> AcctResult<Option<&mut Self::AccountState>>;

    /// Overwrites an existing account entry's state, if it exists.
    ///
    /// This refuses to create new accounts in order to avoid accidents.
    fn update_account_state(&mut self, id: AccountId, state: Self::AccountState) -> AcctResult<()>;

    /// Creates a new account as some ID with some type state, if that ID
    /// doesn't exist, returning the serial.
    fn create_new_account(
        &mut self,
        id: AccountId,
        state: AccountTypeState<Self::AccountState>,
    ) -> AcctResult<AccountSerial>;
}
