use strata_acct_types::{AccountId, AccountSerial, AcctResult};

use crate::{
    account::{AccountTypeState, IAccountState},
    toplevel::IToplevelState,
};

/// Opaque interface for manipulating the ledger state.
///
/// This exists because we want to make this generic across the various
/// different contexts we'll be manipulating state.
pub trait StateAccessor {
    /// Type representing the toplevel chainstate.
    type ToplevelState: IToplevelState;

    /// Type representing ledger account state.
    type AccountState: IAccountState;

    /// Gets the toplevel state.
    fn toplevel(&self) -> &Self::ToplevelState;

    /// Checks if an account exists.
    fn check_account_exists(&self, id: AccountId) -> AcctResult<bool>;

    /// Fetches an account by ID, if it exists.
    fn fetch_account_state(&self, id: AccountId) -> AcctResult<Option<Self::AccountState>>;

    /// Stores an existing account entry's state, if it exists.
    fn store_account_state(&mut self, id: AccountId, state: Self::AccountState) -> AcctResult<()>;

    /// Creates a new account as some ID with some type state, if that ID
    /// doesn't exist, returning the serial.
    fn create_new_account(
        &mut self,
        id: AccountId,
        state: AccountTypeState<Self::AccountState>,
    ) -> AcctResult<AccountSerial>;
}
