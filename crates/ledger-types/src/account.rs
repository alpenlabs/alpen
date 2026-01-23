use strata_acct_types::{AccountSerial, AccountTypeId, AcctResult, BitcoinAmount, Hash, Mmr64};
use strata_predicate::PredicateKey;
use strata_snark_acct_types::{MessageEntry, Seqno};

use crate::coin::Coin;

/// Abstract account state.
pub trait IAccountState: Sized {
    /// Type representing snark account state.
    type SnarkAccountState: ISnarkAccountState;

    /// Gets the account serial.
    fn serial(&self) -> AccountSerial;

    /// Gets the account's balance.
    fn balance(&self) -> BitcoinAmount;

    /// Gets the account type ID.
    fn ty(&self) -> AccountTypeId;

    /// Gets the type state borrowed.
    fn type_state(&self) -> AccountTypeStateRef<'_, Self>;

    /// If we are a snark account, gets a ref to the type state.
    fn as_snark_account(&self) -> AcctResult<&Self::SnarkAccountState>;
}

/// Abstract mutable account state.
pub trait IAccountStateMut: IAccountState {
    /// Mutable snark account state data.
    type SnarkAccountStateMut: ISnarkAccountStateMut;

    /// Adds a coin to this account's balance.
    fn add_balance(&mut self, coin: Coin);

    /// Takes a coin from this account's balance, if funds are available.
    fn take_balance(&mut self, amt: BitcoinAmount) -> AcctResult<Coin>;

    /// If we are a snark, gets a mut ref to the type state.
    fn as_snark_account_mut(&mut self) -> AcctResult<&mut Self::SnarkAccountStateMut>;
}

/// Account state for a newly-created account, which hasn't been assigned a
/// serial yet.
pub struct NewAccountData<T: IAccountState> {
    initial_balance: BitcoinAmount,
    type_state: AccountTypeState<T>,
}

impl<T: IAccountState> Clone for NewAccountData<T>
where
    T::SnarkAccountState: Clone,
{
    fn clone(&self) -> Self {
        Self {
            initial_balance: self.initial_balance,
            type_state: self.type_state.clone(),
        }
    }
}

impl<T: IAccountState> NewAccountData<T> {
    pub fn new(initial_balance: BitcoinAmount, type_state: AccountTypeState<T>) -> Self {
        Self {
            initial_balance,
            type_state,
        }
    }

    pub fn new_empty(type_state: AccountTypeState<T>) -> Self {
        Self::new(BitcoinAmount::zero(), type_state)
    }

    pub fn initial_balance(&self) -> BitcoinAmount {
        self.initial_balance
    }

    pub fn type_state(&self) -> &AccountTypeState<T> {
        &self.type_state
    }

    pub fn into_type_state(self) -> AccountTypeState<T> {
        self.type_state
    }
}

/// Account type state enum.
#[derive(Debug)]
pub enum AccountTypeState<T: IAccountState> {
    /// Empty accounts with no state.
    Empty,

    /// Snark account with snark account state.
    Snark(T::SnarkAccountState),
}

impl<T: IAccountState> Clone for AccountTypeState<T>
where
    T::SnarkAccountState: Clone,
{
    fn clone(&self) -> Self {
        match self {
            Self::Empty => Self::Empty,
            Self::Snark(s) => Self::Snark(s.clone()),
        }
    }
}

/// Borrowed account type state.
#[derive(Copy, Clone, Debug)]
pub enum AccountTypeStateRef<'a, T: IAccountState> {
    Empty,
    Snark(&'a T::SnarkAccountState),
}

/// Mutably borrowed account type state.
#[derive(Debug)]
pub enum AccountTypeStateMut<'a, T: IAccountState> {
    Empty,
    Snark(&'a mut T::SnarkAccountState),
}

/// Abstract snark account state.
pub trait ISnarkAccountState: Sized {
    // Proof state accessors

    /// Gets the verification key for this snark account.
    fn update_vk(&self) -> &PredicateKey;

    /// Gets the update seqno.
    fn seqno(&self) -> Seqno;

    /// Gets the inner state root hash.
    fn inner_state_root(&self) -> Hash;

    /// Gets the index of the next message to be read/processed by this account.
    fn next_inbox_msg_idx(&self) -> u64;

    // Inbox accessors

    /// Gets current the inbox MMR state, which we can use to check proofs
    /// against the state.
    fn inbox_mmr(&self) -> &Mmr64;
}

/// Mutable accessor to snark account state.
pub trait ISnarkAccountStateMut: ISnarkAccountState {
    /// Sets the inner state root unconditionally.
    fn set_proof_state_directly(&mut self, state: Hash, next_read_idx: u64, seqno: Seqno);

    /// Sets an account's inner state, but also taking the update extra data arg
    /// (which is not used directly, but is useful for DA reasons).
    ///
    /// This should also ensure that the seqno always increases.
    fn update_inner_state(
        &mut self,
        inner_state: Hash,
        next_read_idx: u64,
        seqno: Seqno,
        extra_data: &[u8],
    ) -> AcctResult<()>;

    /// Inserts message data into the inbox.  Performs no other operations.
    ///
    /// This is exposed like this so that we can expose the message entry in DA.
    fn insert_inbox_message(&mut self, entry: MessageEntry) -> AcctResult<()>;
}

/// Trait for constructing account states with a serial.
///
/// This is used by generic state accessor wrappers that need to create new
/// accounts but don't have knowledge of the concrete account type.
pub trait IAccountStateConstructible: IAccountState {
    /// Creates a new account state with the given serial, balance, and type state.
    fn new_with_serial(new_acct_data: NewAccountData<Self>, serial: AccountSerial) -> Self;
}
