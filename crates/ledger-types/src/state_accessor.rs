use strata_acct_types::{AccountId, AccountSerial, AcctResult, BitcoinAmount, Mmr64};
use strata_asm_manifest_types::AsmManifest;
use strata_identifiers::{Buf32, EpochCommitment, L1BlockId, L1Height};

use crate::account::{IAccountState, IAccountStateMut, NewAccountData};

/// Opaque interface for manipulating the chainstate, for all of the parts
/// directly under the toplevel state.
///
/// This exists because we want to make this generic across the various
/// different contexts we'll be manipulating state.
pub trait IStateAccessor {
    /// Type representing a ledger account's state for read operations.
    type AccountState: IAccountState;

    /// Same as above, but the mutable view.
    type AccountStateMut: IAccountStateMut;

    // ===== Global state methods =====

    /// Gets the current slot.
    fn cur_slot(&self) -> u64;

    /// Sets the current slot.
    fn set_cur_slot(&mut self, slot: u64);

    // ===== Epochal state methods =====
    // (formerly "L1 view state")

    /// Gets the current epoch.
    fn cur_epoch(&self) -> u32;

    /// Sets the current epoch.
    fn set_cur_epoch(&mut self, epoch: u32);

    /// Last L1 block ID.
    fn last_l1_blkid(&self) -> &L1BlockId;

    /// Last L1 block height.
    fn last_l1_height(&self) -> L1Height;

    /// Appends a new ASM manifest to the accumulator, also updating the last L1
    /// block height and other fields.
    fn append_manifest(&mut self, height: L1Height, mf: AsmManifest);

    /// Gets the field for the epoch that the ASM considers to be valid.
    ///
    /// This is our perspective of the perspective of the last block's ASM
    /// manifest we've accepted.
    fn asm_recorded_epoch(&self) -> &EpochCommitment;

    /// Sets the field for the epoch that the ASM considers to be finalized.
    ///
    /// This is our perspective of the perspective of the last block's ASM
    /// manifest we've accepted.
    fn set_asm_recorded_epoch(&mut self, epoch: EpochCommitment);

    /// Gets the total OL ledger balance.
    fn total_ledger_balance(&self) -> BitcoinAmount;

    /// Sets the total OL ledger balance.
    fn set_total_ledger_balance(&mut self, amt: BitcoinAmount);

    /// Gets the ASM manifests MMR for ledger reference verification.
    fn asm_manifests_mmr(&self) -> &Mmr64;

    // ===== Account methods =====

    /// Checks if an account exists.
    fn check_account_exists(&self, id: AccountId) -> AcctResult<bool>;

    /// Gets a ref to an account, if it exists. For read-only access.
    fn get_account_state(&self, id: AccountId) -> AcctResult<Option<&Self::AccountState>>;

    /// Transactional modification of an account state.
    ///
    /// The closure receives a mutable reference to the account write context and
    /// can modify it. The implementation handles any setup before and cleanup
    /// after the closure returns. Returns whatever the closure returns, wrapped
    /// in `AcctResult`.
    ///
    /// Returns an error if the account doesn't exist.
    fn update_account<R, F>(&mut self, id: AccountId, f: F) -> AcctResult<R>
    where
        F: FnOnce(&mut Self::AccountStateMut) -> R;

    /// Creates a new account as some ID with some type state, if that ID
    /// doesn't exist, assigning it a fresh serial.  Returns the freshly created
    /// serial.
    fn create_new_account(
        &mut self,
        id: AccountId,
        new_acct_data: NewAccountData<Self::AccountState>,
    ) -> AcctResult<AccountSerial>;

    /// Resolves an account serial to an account ID.
    fn find_account_id_by_serial(&self, serial: AccountSerial) -> AcctResult<Option<AccountId>>;

    /// Returns the next account serial that will be assigned when creating a new account.
    fn next_account_serial(&self) -> AccountSerial;

    /// Computes the full state root, using whatever things we've updated.
    // TODO maybe don't use `AcctResult`, actually convert all/most of these to use a new error type
    fn compute_state_root(&self) -> AcctResult<Buf32>;
}
