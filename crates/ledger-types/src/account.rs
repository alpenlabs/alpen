use strata_acct_types::{
    AccountSerial, AccountTypeId, AcctResult, BitcoinAmount, RawAccountTypeId,
};
use strata_snark_acct_types::MessageEntry;

/// Abstract account state.
pub trait IAccountState: Sized {
    /// Type representing snark account state.
    type SnarkAccountState: ISnarkAccountState;

    /// Gets the account serial.
    fn serial(&self) -> AccountSerial;

    /// Gets the account's balance.
    fn balance(&self) -> BitcoinAmount;

    /// Sets the account's balance.
    fn set_balance(&self, amt: BitcoinAmount);

    /// Gets the account raw type ID.
    fn raw_ty(&self) -> AcctResult<RawAccountTypeId>;

    /// Gets the parsed account type ID, if valid.
    fn ty(&self) -> AcctResult<AccountTypeId>;

    /// Gets the account type state, if valid.
    fn get_type_state(&self) -> AcctResult<AccountTypeState<Self>>;

    /// Sets the account type state.
    fn set_type_state(&self, state: AccountTypeState<Self>) -> AcctResult<()>;
}

/// Account type state enum.
pub enum AccountTypeState<T: IAccountState> {
    /// Empty accounts with no state.
    Empty,

    /// Snark account with snark account state.
    Snark(T::SnarkAccountState),
}

/// Abstract snark account state.
pub trait ISnarkAccountState: Sized {
    // TODO accumulator accessors

    /// Gets the next inbox msg index.
    fn get_next_inbox_msg_idx(&self) -> u64;

    /// Gets the update seqno.
    // TODO convert to Seqno
    fn seqno(&self) -> u64;

    /// Gets the inner state root hash.
    fn inner_state_root(&self) -> [u8; 32];

    /// Inserts a message into the inbox.  Performs no other operations.
    fn insert_inbox_message(&mut self, entry: MessageEntry) -> AcctResult<()>;

    /// Increments the sequence number by some amount.
    // TODO convert to Seqno
    fn increment_seqno(&mut self, amt: u64) -> AcctResult<u64>;

    /// Sets the inner state root.
    fn set_inner_state_root(&mut self, state: [u8; 32]);
}
