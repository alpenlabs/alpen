use strata_acct_types::{
    AccountSerial, AccountTypeId, AcctResult, BitcoinAmount, Hash, Mmr64, RawAccountTypeId,
};
use strata_predicate::PredicateKey;
use strata_snark_acct_types::MessageEntry;

use crate::coin::Coin;

type Seqno = u64;

/// Abstract account state.
pub trait IAccountState: Sized + Clone {
    /// Type representing snark account state.
    type SnarkAccountState: ISnarkAccountState;

    /// Gets the account serial.
    fn serial(&self) -> AccountSerial;

    /// Gets the account's balance.
    fn balance(&self) -> BitcoinAmount;

    /// Adds a coin to this account's balance.
    fn add_balance(&mut self, coin: Coin);

    /// Takes a coin from this account's balance, if funds are available.
    fn take_balance(&mut self, amt: BitcoinAmount) -> AcctResult<Coin>;

    /// Gets the account raw type ID.
    fn raw_ty(&self) -> AcctResult<RawAccountTypeId>;

    /// Gets the parsed account type ID, if valid.
    fn ty(&self) -> AcctResult<AccountTypeId>;

    /// Gets the account type state, if valid.
    fn get_type_state(&self) -> AcctResult<AccountTypeState<Self>>;

    /// Gets the mutable account type state, if valid.
    fn get_type_state_mut(&mut self) -> AcctResult<&mut AccountTypeState<Self>>;

    /// Sets the account type state.
    fn set_type_state(&mut self, state: AccountTypeState<Self>) -> AcctResult<()>;
}

/// Account type state enum.
#[derive(Clone, Debug)]
pub enum AccountTypeState<T: IAccountState> {
    /// Empty accounts with no state.
    Empty,

    /// Snark account with snark account state.
    Snark(T::SnarkAccountState),
}

/// Abstract snark account state.
pub trait ISnarkAccountState: Sized {
    /// Verifier key to verify the updates.
    fn verifier_key(&self) -> &PredicateKey;

    // Proof state accessors

    /// Gets the update seqno.
    fn seqno(&self) -> Seqno;

    /// Gets the next inbox index.
    fn next_inbox_idx(&self) -> u64;

    /// Gets the inner state root hash.
    fn inner_state_root(&self) -> Hash;

    /// Sets the inner state root unconditionally.
    fn set_proof_state_directly(&mut self, state: Hash, next_inbox_idx: u64, seqno: Seqno);

    /// Sets an account's inner state, but also taking the update extra data arg
    /// (which is not used directly, but is useful for DA reasons).
    ///
    /// This should also ensure that the seqno always increases.
    fn update_inner_state(
        &mut self,
        state: Hash,
        seqno: Seqno,
        extra_data: &[u8],
    ) -> AcctResult<()>;

    // Inbox accessors

    /// Gets the current inbox MMR state, which we can use to check proofs
    /// against the state.
    fn inbox_mmr(&self) -> &Mmr64;

    /// Inserts message data into the inbox.  Performs no other operations.
    ///
    /// This is exposed like this so that we can expose the message entry in DA.
    fn insert_inbox_message(&mut self, entry: MessageEntry) -> AcctResult<()>;
}

/// Extension trait for abstract snark account state.
pub trait ISnarkAccountStateExt: ISnarkAccountState {
    /// Get the index of the next message that would be inserted into the MMR.
    fn get_next_inbox_msg_idx(&self) -> u64;
}

impl<A: ISnarkAccountState> ISnarkAccountStateExt for A {
    fn get_next_inbox_msg_idx(&self) -> u64 {
        self.inbox_mmr().num_entries()
    }
}
