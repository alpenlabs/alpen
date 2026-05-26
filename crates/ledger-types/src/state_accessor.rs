use strata_acct_types::{AccountId, AccountSerial, BitcoinAmount, Mmr64};
use strata_asm_manifest_types::AsmManifest;
use strata_identifiers::{Buf32, EpochCommitment, L1BlockId, L1Height};

use crate::{
    Coin,
    account::{IAccountState, IAccountStateMut, NewAccountData},
    errors::StateResult,
};

/// Opaque interface for accessing the chainstate, for all of the parts directly
/// under the toplevel state.
///
/// This exists because we want to make this generic across the various
/// different contexts we'll be manipulating state.
pub trait IStateAccessor {
    /// Type representing a ledger account's state for read operations.
    type AccountState: IAccountState;

    // ===== Global state methods =====

    /// Gets the current slot.
    fn cur_slot(&self) -> u64;

    /// Gets the current amount of funds in limbo.
    fn limbo_funds(&self) -> BitcoinAmount;

    // ===== Epochal state methods =====

    /// Gets the current epoch.
    fn cur_epoch(&self) -> u32;

    /// Last L1 block ID.
    fn last_l1_blkid(&self) -> &L1BlockId;

    /// Last L1 block height.
    fn last_l1_height(&self) -> L1Height;

    /// Gets the field for the epoch that the ASM considers to be valid.
    ///
    /// This is our perspective of the perspective of the last block's ASM
    /// manifest we've accepted.
    fn asm_recorded_epoch(&self) -> &EpochCommitment;

    /// Gets the total OL ledger balance.
    fn total_ledger_balance(&self) -> BitcoinAmount;

    /// Gets the OL L1 block refs MMR.
    ///
    /// Indices into this MMR are L1 block heights. The MMR is prefilled at
    /// genesis with zero-hash leaves for heights `0..=genesis_l1_height`, so
    /// callers can use raw L1 heights as MMR leaf indices everywhere.
    fn l1_block_refs_mmr(&self) -> &Mmr64;

    // ===== Account methods =====

    /// Checks if an account exists.
    fn check_account_exists(&self, id: AccountId) -> StateResult<bool>;

    /// Gets a ref to an account, if it exists. For read-only access.
    fn get_account_state(&self, id: AccountId) -> StateResult<Option<&Self::AccountState>>;

    /// Resolves an account serial to an account ID.
    fn find_account_id_by_serial(&self, serial: AccountSerial) -> StateResult<Option<AccountId>>;

    /// Returns the next account serial that will be assigned when creating a new account.
    fn next_account_serial(&self) -> AccountSerial;

    /// Computes the full state root, using whatever things we've updated.
    fn compute_state_root(&self) -> StateResult<Buf32>;
}

/// Like [`IStateAccessor`], but for making writes to the chainstate.
pub trait IStateAccessorMut: IStateAccessor {
    /// Same as above, but the mutable view.
    type AccountStateMut: IAccountStateMut;

    // ===== Global state methods =====

    /// Sets the current slot.
    fn set_cur_slot(&mut self, slot: u64);

    /// Adds a coin to the funds in limbo.
    ///
    /// This uses the [`Coin`] abstraction since it represents a credit.
    fn add_limbo_funds_coin(&mut self, coin: Coin) -> StateResult<()>;

    /// Takes a coin from the funds in limbo.
    ///
    /// This uses the [`Coin`] abstraction since it represents a credit.
    fn take_limbo_funds_coin(&mut self, amt: BitcoinAmount) -> StateResult<Coin>;

    // ===== Epochal state methods =====

    /// Sets the current epoch.
    fn set_cur_epoch(&mut self, epoch: u32);

    /// Appends an accepted ASM manifest's L1 block ref to the accumulator.
    ///
    /// This also updates the last L1 block height and ID.
    fn append_l1_block_ref_from_manifest(&mut self, height: L1Height, mf: AsmManifest);

    /// Sets the field for the epoch that the ASM considers to be finalized.
    ///
    /// This is our perspective of the perspective of the last block's ASM
    /// manifest we've accepted.
    fn set_asm_recorded_epoch(&mut self, epoch: EpochCommitment);

    /// Sets the total OL ledger balance.
    ///
    /// This does not use the [`Coin`] abstraction since it represents an
    /// obligation to fulfill, not a credit.
    fn set_total_ledger_balance(&mut self, amt: BitcoinAmount);

    // ===== Account methods =====

    /// Transactional modification of an account state.
    ///
    /// The closure receives a mutable reference to the account write context and
    /// can modify it. The implementation handles any setup before and cleanup
    /// after the closure returns. Returns whatever the closure returns, wrapped
    /// in `StateResult`.
    ///
    /// Returns an error if the account doesn't exist.
    fn update_account<R, F>(&mut self, id: AccountId, f: F) -> StateResult<R>
    where
        F: FnOnce(&mut Self::AccountStateMut) -> R;

    /// Creates a new account as some ID with some type state, if that ID
    /// doesn't exist, assigning it a fresh serial.  Returns the freshly created
    /// serial.
    fn create_new_account(
        &mut self,
        id: AccountId,
        new_acct_data: NewAccountData,
    ) -> StateResult<AccountSerial>;
}
