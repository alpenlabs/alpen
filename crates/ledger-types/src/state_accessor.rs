use strata_acct_types::{AccountId, AccountSerial, AcctResult, BitcoinAmount, Hash};
use strata_asm_common::AsmManifest;
use strata_identifiers::{Buf32, EpochCommitment, L1Height};
use strata_snark_acct_types::{MessageEntry, Seqno};

use crate::{
    Coin,
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
    type AccountState: IAccountState;

    /// Gets a ref to the global state.
    fn global(&self) -> &Self::GlobalState;

    // GLOBAL MODIFIERS

    /// Sets the current slot.
    fn set_cur_slot(&mut self, slot: u64);

    /// Gets a ref to the L1 view state.
    fn l1_view(&self) -> &Self::L1ViewState;

    // L1 View MODIFIERS

    /// Sets the current epoch.
    fn set_cur_epoch(&mut self, epoch: u32);

    /// Appends a new ASM manifest to the accumulator, also updating the last L1
    /// block height and other fields.
    fn append_manifest(&mut self, height: L1Height, mf: AsmManifest);

    /// Sets the field for the epoch that the ASM considers to be finalized.
    ///
    /// This is our perspective of the perspective of the last block's ASM
    /// manifest we've accepted.
    fn set_asm_recorded_epoch(&mut self, epoch: EpochCommitment);

    /// Checks if an account exists.
    fn check_account_exists(&self, id: AccountId) -> AcctResult<bool>;

    /// Gets a ref to an account, if it exists.
    fn get_account_state(&self, id: AccountId) -> AcctResult<Option<&Self::AccountState>>;

    // Account MODIFIERS

    /// Adds a coin to this account's balance.
    fn add_balance(&mut self, acct_id: AccountId, coin: Coin) -> AcctResult<()>;

    /// Takes a coin from this account's balance, if funds are available.
    fn take_balance(&mut self, acct_id: AccountId, amt: BitcoinAmount) -> AcctResult<Coin>;

    /// Sets the inner state root unconditionally.
    ///
    /// # Note
    /// Returns error for non-snark account.
    fn set_proof_state_directly(
        &mut self,
        acct_id: AccountId,
        state: Hash,
        next_read_idx: u64,
        seqno: Seqno,
    ) -> AcctResult<()>;

    /// Inserts message data into the inbox.  Performs no other operations.
    ///
    /// This is exposed like this so that we can expose the message entry in DA.
    ///
    /// # Note
    /// Returns error for non-snark account
    fn insert_inbox_message(&mut self, acct_id: AccountId, entry: MessageEntry) -> AcctResult<()>;

    /// Overwrites an existing account entry's state, if it exists.
    ///
    /// This refuses to create new accounts in order to avoid accidents like
    /// screwing up serials.
    // TODO: might not be necessary with the above modifiers.
    fn update_account_state(&mut self, id: AccountId, state: Self::AccountState) -> AcctResult<()>;

    /// Creates a new account as some ID with some type state, if that ID
    /// doesn't exist, assigning it a fresh serial.  Returns the freshly created
    /// serial.
    fn create_new_account(
        &mut self,
        id: AccountId,
        state: AccountTypeState<Self::AccountState>,
    ) -> AcctResult<AccountSerial>;

    /// Resolves an account serial to an account ID.
    fn find_account_id_by_serial(&self, serial: AccountSerial) -> AcctResult<Option<AccountId>>;

    /// Gets the next available serial number.
    ///
    /// This returns what serial would be assigned to the next created account.
    /// This does not consume or increment the serial.
    fn get_next_serial(&self) -> AccountSerial;

    /// Computes the full state root, using whatever things we've updated.
    // TODO don't use `AcctResult`, actually convert all/most of these to use a new error type
    fn compute_state_root(&self) -> AcctResult<Buf32>;
}
