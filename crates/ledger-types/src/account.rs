use strata_acct_types::{AccountSerial, AccountTypeId, BitcoinAmount, Hash, MessageEntry, Mmr64};
use strata_predicate::PredicateKey;
use strata_snark_acct_types::Seqno;

use crate::{coin::Coin, errors::StateResult};

/// Abstract account state.
pub trait IAccountState: Clone + Sized {
    /// Type representing snark account state.
    type SnarkAccountState: ISnarkAccountState;

    // Constructor.

    /// Creates a new account state with the given serial, balance, and type state.
    ///
    /// This is just a dumb piece of data, it does not insert it into any state
    /// tree or anything.
    fn new_with_serial(new_acct_data: NewAccountData, serial: AccountSerial) -> Self;

    // Accessors.

    /// Gets the account serial.
    fn serial(&self) -> AccountSerial;

    /// Gets the account's balance.
    fn balance(&self) -> BitcoinAmount;

    /// Gets the account type ID.
    fn ty(&self) -> AccountTypeId;

    /// Gets the type state borrowed.
    fn type_state(&self) -> AccountTypeStateRef<'_, Self>;

    /// If we are a snark account, gets a ref to the type state.
    fn as_snark_account(&self) -> StateResult<&Self::SnarkAccountState>;
}

/// Abstract mutable account state.
pub trait IAccountStateMut: IAccountState {
    /// Mutable snark account state data.
    type SnarkAccountStateMut: ISnarkAccountStateMut;

    /// Adds a coin to this account's balance.
    fn add_balance(&mut self, coin: Coin);

    /// Takes a coin from this account's balance, if funds are available.
    fn take_balance(&mut self, amt: BitcoinAmount) -> StateResult<Coin>;

    /// If we are a snark, gets a mut ref to the type state.
    fn as_snark_account_mut(&mut self) -> StateResult<&mut Self::SnarkAccountStateMut>;
}

/// Type-specific initialization state for new accounts.
#[derive(Clone, Debug)]
pub enum NewAccountTypeState {
    /// Empty account with no type state.
    Empty,

    /// Snark account with initial snark parameters.
    Snark {
        /// Update verification key.
        update_vk: PredicateKey,
        /// Initial inner state root.
        initial_state_root: Hash,
    },
}

/// Account state for a newly-created account, which hasn't been assigned a
/// serial yet.
#[derive(Clone, Debug)]
pub struct NewAccountData {
    initial_balance: BitcoinAmount,
    type_state: NewAccountTypeState,
}

impl NewAccountData {
    pub fn new(initial_balance: BitcoinAmount, type_state: NewAccountTypeState) -> Self {
        Self {
            initial_balance,
            type_state,
        }
    }

    pub fn new_empty(type_state: NewAccountTypeState) -> Self {
        Self::new(BitcoinAmount::zero(), type_state)
    }

    /// Creates a new snark account with the given balance, verification key, and initial state
    /// root.
    pub fn new_snark(
        initial_balance: BitcoinAmount,
        update_vk: PredicateKey,
        initial_state_root: Hash,
    ) -> Self {
        Self::new(
            initial_balance,
            NewAccountTypeState::Snark {
                update_vk,
                initial_state_root,
            },
        )
    }

    pub fn initial_balance(&self) -> BitcoinAmount {
        self.initial_balance
    }

    pub fn type_state(&self) -> &NewAccountTypeState {
        &self.type_state
    }

    pub fn into_type_state(self) -> NewAccountTypeState {
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

impl<T: IAccountState> Clone for AccountTypeState<T> {
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
pub trait ISnarkAccountState: Clone + Sized {
    // Constructor.

    /// Builds a fresh snark state from the update predicate key and initial root.
    fn new_fresh(update_vk: PredicateKey, initial_state_root: Hash) -> Self;

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
    ) -> StateResult<()>;

    /// Inserts message data into the inbox.  Performs no other operations.
    ///
    /// This is exposed like this so that we can expose the message entry in DA.
    fn insert_inbox_message(&mut self, entry: MessageEntry) -> StateResult<()>;

    /// Replaces the predicate key (verification key) used to verify future
    /// updates to this snark account.
    ///
    /// Does not touch proof state, seqno, or inbox.
    fn set_update_vk(&mut self, new_vk: PredicateKey);
}
