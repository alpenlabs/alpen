//! Generic behavioral tests shared across the state access layers.
//!
//! The wrapper layers in this crate ([`BatchDiffState`], [`WriteTrackingState`],
//! [`IndexerState`], [`DaAccumulatingState`]) all expose the same
//! [`IStateAccessor`]/[`IStateAccessorMut`] surface over a base layer. The tests
//! that exercise only that surface behave identically regardless of which
//! wrapper is under test, so they are defined once here and instantiated per
//! layer stack via [`impl_read_layer_tests!`] and [`impl_mut_layer_tests!`],
//! mirroring the `db/tests` + `db/store-sled` pattern.
//!
//! Each behavior is a plain generic function taking the [`Fixture`] it runs
//! against and the layer under test. A layer stack is described by a
//! *stack-builder macro* living next to the layer it is rooted at, which
//! expands to the `let` chain that assembles the stack:
//!
//! ```ignore
//! macro_rules! build_wt_over_batch_diff {
//!     ($base:expr, $layer:ident) => {
//!         let batches = [WriteBatch::default(), WriteBatch::default()];
//!         let diff = BatchDiffState::new($base, &batches);
//!         let $layer = WriteTrackingState::new_empty(&diff);
//!     };
//!     // ...plus the same arm binding `mut $layer`.
//! }
//!
//! impl_read_layer_tests!(build_wt_over_batch_diff);
//! impl_mut_layer_tests!(build_wt_over_batch_diff);
//! ```
//!
//! The builder is expanded in statement position inside the generated test
//! function, so intermediate rungs stay alive for the whole test. That matters
//! because [`BatchDiffState`] and [`WriteTrackingState`] borrow the layer
//! beneath them: a value-returning factory could not build such a stack, since
//! the inner rung would be dropped before the layer was returned.
//!
//! Tests that assert on layer-specific internals (e.g. batch extraction,
//! captured indexer writes, DA blob encoding) stay in the individual layer
//! modules.
//!
//! [`BatchDiffState`]: crate::BatchDiffState
//! [`WriteTrackingState`]: crate::WriteTrackingState
//! [`IndexerState`]: crate::IndexerState
//! [`DaAccumulatingState`]: crate::DaAccumulatingState

use std::iter;

use strata_acct_types::{AccountId, AccountTypeId, BitcoinAmount, L1BlockRecord};
use strata_identifiers::{AccountSerial, Buf32, EpochCommitment, L1BlockId, L1Height, OLBlockId};
use strata_ol_state_types::{
    Coin, IAccountState, IAccountStateMut, ISnarkAccountState, ISnarkAccountStateMut,
    IStateAccessor, IStateAccessorMut, NewAccountData, NewAccountTypeState, StateError,
};
use strata_snark_acct_types::Seqno;

use crate::{memory_state_layer::MemoryStateBaseLayer, test_utils::*};

/// Seed of the first account id that the [`Fixture`] guarantees is unused.
///
/// Far enough from the fixture account's own seed that no behavior can collide
/// with it by accident.
const FIRST_UNUSED_ACCOUNT_SEED: u8 = 0x40;

/// Base state that every shared behavior runs against.
///
/// A single fixture shape is used throughout so that a stack-builder macro only
/// has to know how to build a layer over [`base`](Self::base). The base is
/// deliberately non-trivial — a non-genesis epoch and slot, a pre-existing snark
/// account, seeded epochal fields and buffered pending ASM logs — so that every
/// layer's read fall-through is exercised by default rather than comparing
/// genesis defaults against genesis defaults.
pub(crate) struct Fixture {
    base: MemoryStateBaseLayer,
    account_id: AccountId,
    serial: AccountSerial,
}

impl Fixture {
    /// Epoch the base is seeded at.
    pub(crate) const EPOCH: u32 = 5;

    /// Slot the base is seeded at.
    pub(crate) const SLOT: u64 = 100;

    /// Returns the balance of the snark account that already exists in the base.
    pub(crate) fn account_balance() -> BitcoinAmount {
        BitcoinAmount::try_from(1_000).expect("fixture amount must be valid")
    }

    /// Inner state root seed of the account that already exists in the base.
    pub(crate) const ACCOUNT_STATE_SEED: u8 = 1;

    /// Returns the limbo funds seeded on the base.
    pub(crate) fn limbo_funds() -> BitcoinAmount {
        BitcoinAmount::try_from(2_000).expect("fixture amount must be valid")
    }

    /// Returns the total ledger balance seeded on the base.
    pub(crate) fn total_ledger_balance() -> BitcoinAmount {
        BitcoinAmount::try_from(9_000).expect("fixture amount must be valid")
    }

    /// Number of pending ASM logs buffered on the base.
    pub(crate) const PENDING_LOGS: usize = 3;

    /// Builds the fixture base state.
    pub(crate) fn new() -> Self {
        let account_id = test_account_id(1);
        let mut base = new_layer_at(Self::EPOCH, Self::SLOT);

        let serial = base
            .create_new_account(
                account_id,
                test_new_snark_account_data(
                    &test_snark_account_state(Self::ACCOUNT_STATE_SEED),
                    Self::account_balance(),
                ),
            )
            .expect("fixture: create account");

        base.add_limbo_funds_coin(Coin::new_unchecked(Self::limbo_funds()))
            .expect("fixture: add limbo funds");
        base.set_total_ledger_balance(Self::total_ledger_balance());
        base.set_asm_recorded_epoch(Self::asm_recorded_epoch());

        // Indices into the L1 block refs MMR are L1 heights, so the record has
        // to be appended at the height matching the current entry count.
        let height = L1Height::from(base.l1_block_refs_mmr().num_entries() as u32);
        base.append_l1_block_rec(height, L1BlockRecord::new([1u8; 32], [2u8; 32]));

        for i in 0..Self::PENDING_LOGS {
            base.try_append_pending_asm_log(test_pending_asm_log(i as u8))
                .expect("fixture: append pending log");
        }

        Self {
            base,
            account_id,
            serial,
        }
    }

    /// The ASM-recorded epoch seeded on the base.
    pub(crate) fn asm_recorded_epoch() -> EpochCommitment {
        EpochCommitment::new(
            Self::EPOCH - 1,
            Self::SLOT - 10,
            OLBlockId::from(Buf32::from([3u8; 32])),
        )
    }

    /// The base state that the layer under test is built over.
    pub(crate) fn base(&self) -> &MemoryStateBaseLayer {
        &self.base
    }

    /// Id of the snark account that already exists in the base.
    pub(crate) fn account_id(&self) -> AccountId {
        self.account_id
    }

    /// Serial of the account that already exists in the base.
    pub(crate) fn serial(&self) -> AccountSerial {
        self.serial
    }

    /// Returns an account id that is guaranteed to be absent from the base.
    ///
    /// Each `n` yields a distinct id, so a single behavior can create several
    /// accounts without collisions.
    pub(crate) fn unused_account_id(n: u8) -> AccountId {
        test_account_id(FIRST_UNUSED_ACCOUNT_SEED + n)
    }
}

/// Returns `amt` increased by `sats`.
fn plus_sats(amt: BitcoinAmount, sats: u64) -> BitcoinAmount {
    BitcoinAmount::try_from(amt.to_sat() + sats)
        .expect("amount must not exceed the Bitcoin money supply")
}

/// Returns `amt` decreased by `sats`.
fn minus_sats(amt: BitcoinAmount, sats: u64) -> BitcoinAmount {
    BitcoinAmount::try_from(amt.to_sat() - sats)
        .expect("amount must not exceed the Bitcoin money supply")
}

// =============================================================================
// Read behaviors (IStateAccessor)
// =============================================================================

/// Reading an account present only in the base falls through to it.
pub(crate) fn read_falls_back_to_base<S: IStateAccessor>(fx: &Fixture, layer: &S) {
    let account = layer.get_account_state(fx.account_id()).unwrap().unwrap();
    assert_eq!(account.serial(), fx.serial());
    assert_eq!(account.balance(), Fixture::account_balance());
}

/// `check_account_exists` reflects presence in the base.
pub(crate) fn check_account_exists_falls_back_to_base<S: IStateAccessor>(fx: &Fixture, layer: &S) {
    assert!(layer.check_account_exists(fx.account_id()).unwrap());
    assert!(
        !layer
            .check_account_exists(Fixture::unused_account_id(0))
            .unwrap()
    );
}

/// With no writes of its own, every read through the layer reflects the base.
pub(crate) fn reads_all_fields_from_base<S: IStateAccessor>(fx: &Fixture, layer: &S) {
    let base = fx.base();

    assert_eq!(layer.cur_slot(), base.cur_slot());
    assert_eq!(layer.limbo_funds(), base.limbo_funds());
    assert_eq!(layer.cur_epoch(), base.cur_epoch());
    assert_eq!(layer.last_l1_blkid(), base.last_l1_blkid());
    assert_eq!(layer.last_l1_height(), base.last_l1_height());
    assert_eq!(layer.asm_recorded_epoch(), base.asm_recorded_epoch());
    assert_eq!(layer.total_ledger_balance(), base.total_ledger_balance());
    assert_eq!(
        layer.l1_block_refs_mmr().num_entries(),
        base.l1_block_refs_mmr().num_entries()
    );
    assert_eq!(layer.pending_asm_logs_len(), base.pending_asm_logs_len());
    assert_eq!(layer.pending_asm_logs_full(), base.pending_asm_logs_full());
    assert_eq!(layer.next_account_serial(), base.next_account_serial());

    for idx in 0..base.pending_asm_logs_len() {
        let from_layer = layer.get_pending_asm_log(idx).expect("layer log");
        let from_base = base.get_pending_asm_log(idx).expect("base log");
        assert_eq!(from_layer.height(), from_base.height());
    }
    assert!(
        layer
            .get_pending_asm_log(base.pending_asm_logs_len())
            .is_none()
    );

    // The fixture seeds all of these away from their genesis defaults, so the
    // comparisons above are meaningful rather than trivially true.
    assert_eq!(layer.cur_epoch(), Fixture::EPOCH);
    assert_eq!(layer.cur_slot(), Fixture::SLOT);
    assert_eq!(layer.limbo_funds(), Fixture::limbo_funds());
    assert_eq!(
        layer.total_ledger_balance(),
        Fixture::total_ledger_balance()
    );
    assert_eq!(*layer.asm_recorded_epoch(), Fixture::asm_recorded_epoch());
    assert_eq!(layer.pending_asm_logs_len(), Fixture::PENDING_LOGS);
    assert!(!layer.pending_asm_logs_full());
}

/// A base account can be resolved by its serial.
pub(crate) fn find_account_by_serial_from_base<S: IStateAccessor>(fx: &Fixture, layer: &S) {
    assert_eq!(
        layer.find_account_id_by_serial(fx.serial()).unwrap(),
        Some(fx.account_id())
    );
}

/// With no writes, the layer's state root matches the base's.
pub(crate) fn state_root_matches_base_with_no_writes<S: IStateAccessor>(fx: &Fixture, layer: &S) {
    assert_eq!(
        layer.compute_state_root().unwrap(),
        fx.base().compute_state_root().unwrap()
    );
}

/// Reading an account that exists in neither the layer nor the base returns
/// `None` (as opposed to erroring).
pub(crate) fn get_missing_account_returns_none<S: IStateAccessor>(_fx: &Fixture, layer: &S) {
    assert!(
        layer
            .get_account_state(Fixture::unused_account_id(0))
            .unwrap()
            .is_none()
    );
}

/// Resolving a serial that was never assigned returns `None`.
pub(crate) fn find_serial_returns_none_for_unknown<S: IStateAccessor>(_fx: &Fixture, layer: &S) {
    assert!(
        layer
            .find_account_id_by_serial(AccountSerial::from(9999))
            .unwrap()
            .is_none()
    );
}

// =============================================================================
// Write behaviors (IStateAccessorMut)
// =============================================================================

/// Updating an account is visible through the layer but leaves the base intact.
pub(crate) fn update_account_isolated_from_base<S: IStateAccessorMut>(fx: &Fixture, layer: &mut S) {
    layer
        .update_account(fx.account_id(), |acct| {
            acct.add_balance(Coin::new_unchecked(
                BitcoinAmount::try_from(500)
                    .expect("amount must not exceed the Bitcoin money supply"),
            ));
        })
        .unwrap();

    let account = layer.get_account_state(fx.account_id()).unwrap().unwrap();
    assert_eq!(
        account.balance(),
        plus_sats(Fixture::account_balance(), 500)
    );

    // Base is untouched.
    let base_account = fx
        .base()
        .get_account_state(fx.account_id())
        .unwrap()
        .unwrap();
    assert_eq!(base_account.balance(), Fixture::account_balance());
}

/// Repeated updates to the same account accumulate on the layer's copy.
pub(crate) fn repeated_update_accumulates<S: IStateAccessorMut>(fx: &Fixture, layer: &mut S) {
    layer
        .update_account(fx.account_id(), |acct| {
            acct.add_balance(Coin::new_unchecked(
                BitcoinAmount::try_from(500)
                    .expect("amount must not exceed the Bitcoin money supply"),
            ));
        })
        .unwrap();
    layer
        .update_account(fx.account_id(), |acct| {
            acct.add_balance(Coin::new_unchecked(
                BitcoinAmount::try_from(100)
                    .expect("amount must not exceed the Bitcoin money supply"),
            ));
        })
        .unwrap();

    let account = layer.get_account_state(fx.account_id()).unwrap().unwrap();
    assert_eq!(
        account.balance(),
        plus_sats(Fixture::account_balance(), 600)
    );
}

/// Taking balance from an account debits it and yields a [`Coin`] of the taken
/// amount.
pub(crate) fn take_balance_reads_back<S: IStateAccessorMut>(fx: &Fixture, layer: &mut S) {
    let coin = layer
        .update_account(fx.account_id(), |acct| {
            acct.take_balance(
                BitcoinAmount::try_from(300)
                    .expect("amount must not exceed the Bitcoin money supply"),
            )
        })
        .unwrap()
        .unwrap();
    coin.safely_consume_unchecked();

    let account = layer.get_account_state(fx.account_id()).unwrap().unwrap();
    assert_eq!(
        account.balance(),
        minus_sats(Fixture::account_balance(), 300)
    );
}

/// Taking an account's entire balance leaves it at zero rather than erroring.
pub(crate) fn take_balance_exact_empties_account<S: IStateAccessorMut>(
    fx: &Fixture,
    layer: &mut S,
) {
    let coin = layer
        .update_account(fx.account_id(), |acct| {
            acct.take_balance(Fixture::account_balance())
        })
        .unwrap()
        .unwrap();
    assert_eq!(coin.amt(), Fixture::account_balance());
    coin.safely_consume_unchecked();

    let account = layer.get_account_state(fx.account_id()).unwrap().unwrap();
    assert_eq!(account.balance(), BitcoinAmount::default());
}

/// Taking more balance than an account holds errors with
/// [`StateError::InsufficientBalance`] and leaves the balance unchanged.
pub(crate) fn take_balance_insufficient_errors<S: IStateAccessorMut>(fx: &Fixture, layer: &mut S) {
    let result = layer
        .update_account(fx.account_id(), |acct| {
            acct.take_balance(plus_sats(Fixture::account_balance(), 1))
        })
        .unwrap();
    assert!(matches!(
        result,
        Err(StateError::InsufficientBalance { .. })
    ));

    let account = layer.get_account_state(fx.account_id()).unwrap().unwrap();
    assert_eq!(account.balance(), Fixture::account_balance());
}

/// A freshly created account is visible through the layer and resolvable by
/// serial.
pub(crate) fn create_account_visible<S: IStateAccessorMut>(_fx: &Fixture, layer: &mut S) {
    let account_id = Fixture::unused_account_id(0);
    let new_acct = test_new_snark_account_data(
        &test_snark_account_state(2),
        BitcoinAmount::try_from(5000).expect("amount must not exceed the Bitcoin money supply"),
    );
    let serial = layer.create_new_account(account_id, new_acct).unwrap();

    assert!(layer.check_account_exists(account_id).unwrap());
    let account = layer.get_account_state(account_id).unwrap().unwrap();
    assert_eq!(account.serial(), serial);
    assert_eq!(
        account.balance(),
        BitcoinAmount::try_from(5000).expect("amount must not exceed the Bitcoin money supply")
    );
    assert_eq!(
        layer.find_account_id_by_serial(serial).unwrap(),
        Some(account_id)
    );
}

/// A freshly created empty (non-snark) account reports the empty type, and
/// asking for its snark state errors with [`StateError::MismatchedAcctType`].
pub(crate) fn create_empty_account<S: IStateAccessorMut>(_fx: &Fixture, layer: &mut S) {
    let account_id = Fixture::unused_account_id(0);
    let new_acct = NewAccountData::new(
        BitcoinAmount::try_from(42).expect("amount must not exceed the Bitcoin money supply"),
        NewAccountTypeState::Empty,
    );
    layer.create_new_account(account_id, new_acct).unwrap();

    let account = layer.get_account_state(account_id).unwrap().unwrap();
    assert_eq!(account.ty(), AccountTypeId::Empty);
    assert_eq!(
        account.balance(),
        BitcoinAmount::try_from(42).expect("amount must not exceed the Bitcoin money supply")
    );
    assert!(matches!(
        account.as_snark_account(),
        Err(StateError::MismatchedAcctType { .. })
    ));
}

/// An account created through the layer can be updated through it afterwards,
/// with its type state preserved across the update.
pub(crate) fn create_then_update_account<S: IStateAccessorMut>(_fx: &Fixture, layer: &mut S) {
    let account_id = Fixture::unused_account_id(0);
    let snark_state = test_snark_account_state(2);
    let new_acct = test_new_snark_account_data(
        &snark_state,
        BitcoinAmount::try_from(1_000).expect("amount must not exceed the Bitcoin money supply"),
    );
    layer.create_new_account(account_id, new_acct).unwrap();

    layer
        .update_account(account_id, |acct| {
            acct.add_balance(Coin::new_unchecked(
                BitcoinAmount::try_from(250)
                    .expect("amount must not exceed the Bitcoin money supply"),
            ));
        })
        .unwrap();

    let account = layer.get_account_state(account_id).unwrap().unwrap();
    assert_eq!(
        account.balance(),
        BitcoinAmount::try_from(1_250).expect("amount must not exceed the Bitcoin money supply")
    );
    assert_eq!(
        account.as_snark_account().unwrap().inner_state_root(),
        snark_state.inner_state_root()
    );
}

/// Creating several accounts assigns sequential serials and advances
/// [`next_account_serial`](IStateAccessor::next_account_serial) accordingly.
pub(crate) fn next_serial_advances_across_creates<S: IStateAccessorMut>(
    _fx: &Fixture,
    layer: &mut S,
) {
    let first_serial: u32 = layer.next_account_serial().into();

    let acct_a = Fixture::unused_account_id(0);
    let acct_b = Fixture::unused_account_id(1);
    let serial_a: u32 = layer
        .create_new_account(
            acct_a,
            test_new_snark_account_data(
                &test_snark_account_state(2),
                BitcoinAmount::try_from(1)
                    .expect("amount must not exceed the Bitcoin money supply"),
            ),
        )
        .unwrap()
        .into();
    let serial_b: u32 = layer
        .create_new_account(
            acct_b,
            test_new_snark_account_data(
                &test_snark_account_state(3),
                BitcoinAmount::try_from(2)
                    .expect("amount must not exceed the Bitcoin money supply"),
            ),
        )
        .unwrap()
        .into();

    assert_eq!(serial_a, first_serial);
    assert_eq!(serial_b, first_serial + 1);

    let next_serial: u32 = layer.next_account_serial().into();
    assert_eq!(next_serial, first_serial + 2);

    // Both are independently resolvable.
    assert_eq!(
        layer
            .find_account_id_by_serial(AccountSerial::from(serial_a))
            .unwrap(),
        Some(acct_a)
    );
    assert_eq!(
        layer
            .find_account_id_by_serial(AccountSerial::from(serial_b))
            .unwrap(),
        Some(acct_b)
    );
}

/// Recreating an account that this layer created itself is a hard error
/// ([`StateError::AccountExists`]), not a silent overwrite.
pub(crate) fn create_duplicate_account_errors<S: IStateAccessorMut>(_fx: &Fixture, layer: &mut S) {
    let account_id = Fixture::unused_account_id(0);
    layer
        .create_new_account(
            account_id,
            test_new_snark_account_data(
                &test_snark_account_state(2),
                BitcoinAmount::try_from(100)
                    .expect("amount must not exceed the Bitcoin money supply"),
            ),
        )
        .unwrap();

    let result = layer.create_new_account(
        account_id,
        test_new_snark_account_data(
            &test_snark_account_state(3),
            BitcoinAmount::try_from(200).expect("amount must not exceed the Bitcoin money supply"),
        ),
    );
    assert!(matches!(result, Err(StateError::AccountExists(_))));
}

/// Recreating an account that exists only in the base is a hard error too, and
/// leaves both the existing account and the serial counter alone.
///
/// This is the case the duplicate guard actually exists for: the layer has no
/// record of the account, so the check has to fall through to the base.
pub(crate) fn create_duplicate_of_base_account_errors<S: IStateAccessorMut>(
    fx: &Fixture,
    layer: &mut S,
) {
    let serial_before = layer.next_account_serial();

    let result = layer.create_new_account(
        fx.account_id(),
        test_new_snark_account_data(
            &test_snark_account_state(2),
            BitcoinAmount::try_from(999).expect("amount must not exceed the Bitcoin money supply"),
        ),
    );
    assert!(matches!(result, Err(StateError::AccountExists(_))));

    assert_eq!(layer.next_account_serial(), serial_before);
    let account = layer.get_account_state(fx.account_id()).unwrap().unwrap();
    assert_eq!(account.serial(), fx.serial());
    assert_eq!(account.balance(), Fixture::account_balance());
    assert_eq!(
        account.as_snark_account().unwrap().inner_state_root(),
        test_hash(Fixture::ACCOUNT_STATE_SEED)
    );
}

/// [`set_proof_state`](ISnarkAccountStateMut::set_proof_state) reads back via
/// [`inner_state_root`](ISnarkAccountState::inner_state_root) and
/// [`seqno`](ISnarkAccountState::seqno).
pub(crate) fn set_proof_state_roundtrip<S: IStateAccessorMut>(fx: &Fixture, layer: &mut S) {
    let inner_state = Buf32::from([42u8; 32]);
    let next_read_idx = 3u64;
    let seqno = Seqno::from(7u64);
    layer
        .update_account(fx.account_id(), |acct| {
            acct.as_snark_account_mut()
                .unwrap()
                .set_proof_state(inner_state, next_read_idx, seqno)
        })
        .unwrap();

    let account = layer.get_account_state(fx.account_id()).unwrap().unwrap();
    let snark = account.as_snark_account().unwrap();
    assert_eq!(snark.inner_state_root(), inner_state);
    assert_eq!(snark.seqno(), seqno);
}

// -----------------------------------------------------------------------------
// Simple state write -> read-back roundtrips
//
// One `roundtrip_*` test per simple setter on `IStateAccessorMut`: perform the
// write, then assert the matching getter reads the value back.
// -----------------------------------------------------------------------------

/// [`set_cur_slot`](IStateAccessorMut::set_cur_slot) reads back via
/// [`cur_slot`](IStateAccessor::cur_slot).
pub(crate) fn roundtrip_cur_slot<S: IStateAccessorMut>(_fx: &Fixture, layer: &mut S) {
    assert_eq!(layer.cur_slot(), Fixture::SLOT);
    layer.set_cur_slot(Fixture::SLOT + 42);
    assert_eq!(layer.cur_slot(), Fixture::SLOT + 42);
}

/// [`set_cur_epoch`](IStateAccessorMut::set_cur_epoch) reads back via
/// [`cur_epoch`](IStateAccessor::cur_epoch).
pub(crate) fn roundtrip_cur_epoch<S: IStateAccessorMut>(_fx: &Fixture, layer: &mut S) {
    assert_eq!(layer.cur_epoch(), Fixture::EPOCH);
    layer.set_cur_epoch(Fixture::EPOCH + 1);
    assert_eq!(layer.cur_epoch(), Fixture::EPOCH + 1);
}

/// [`add_limbo_funds_coin`](IStateAccessorMut::add_limbo_funds_coin) and
/// [`take_limbo_funds_coin`](IStateAccessorMut::take_limbo_funds_coin) read back
/// via [`limbo_funds`](IStateAccessor::limbo_funds).
pub(crate) fn roundtrip_limbo_funds<S: IStateAccessorMut>(_fx: &Fixture, layer: &mut S) {
    assert_eq!(layer.limbo_funds(), Fixture::limbo_funds());

    layer
        .add_limbo_funds_coin(Coin::new_unchecked(
            BitcoinAmount::try_from(1_000)
                .expect("amount must not exceed the Bitcoin money supply"),
        ))
        .unwrap();
    assert_eq!(
        layer.limbo_funds(),
        plus_sats(Fixture::limbo_funds(), 1_000)
    );

    let taken = layer
        .take_limbo_funds_coin(
            BitcoinAmount::try_from(400).expect("amount must not exceed the Bitcoin money supply"),
        )
        .unwrap();
    taken.safely_consume_unchecked();
    assert_eq!(layer.limbo_funds(), plus_sats(Fixture::limbo_funds(), 600));
}

/// Taking more limbo funds than are available errors with
/// [`StateError::InsufficientLimboFunds`] and leaves the balance unchanged.
pub(crate) fn take_limbo_funds_insufficient_errors<S: IStateAccessorMut>(
    _fx: &Fixture,
    layer: &mut S,
) {
    let result = layer.take_limbo_funds_coin(plus_sats(Fixture::limbo_funds(), 1));
    assert!(matches!(
        result,
        Err(StateError::InsufficientLimboFunds { .. })
    ));
    assert_eq!(layer.limbo_funds(), Fixture::limbo_funds());
}

/// Adding limbo funds that would overflow the accumulator errors with
/// [`StateError::LimboFundsOverflow`], consumes the rejected [`Coin`] rather
/// than panicking on drop, and leaves the balance unchanged.
pub(crate) fn add_limbo_funds_overflow_errors<S: IStateAccessorMut>(_fx: &Fixture, layer: &mut S) {
    let max_money = BitcoinAmount::try_from(21_000_000 * 100_000_000)
        .expect("maximum Bitcoin money supply must be valid");
    let result = layer.add_limbo_funds_coin(Coin::new_unchecked(max_money));
    assert!(matches!(result, Err(StateError::LimboFundsOverflow { .. })));
    assert_eq!(layer.limbo_funds(), Fixture::limbo_funds());
}

/// [`set_total_ledger_balance`](IStateAccessorMut::set_total_ledger_balance)
/// reads back via [`total_ledger_balance`](IStateAccessor::total_ledger_balance).
pub(crate) fn roundtrip_total_ledger_balance<S: IStateAccessorMut>(_fx: &Fixture, layer: &mut S) {
    assert_eq!(
        layer.total_ledger_balance(),
        Fixture::total_ledger_balance()
    );
    layer.set_total_ledger_balance(
        BitcoinAmount::try_from(1_000_000)
            .expect("amount must not exceed the Bitcoin money supply"),
    );
    assert_eq!(
        layer.total_ledger_balance(),
        BitcoinAmount::try_from(1_000_000)
            .expect("amount must not exceed the Bitcoin money supply")
    );
}

/// [`set_asm_recorded_epoch`](IStateAccessorMut::set_asm_recorded_epoch) reads
/// back via [`asm_recorded_epoch`](IStateAccessor::asm_recorded_epoch).
pub(crate) fn roundtrip_asm_recorded_epoch<S: IStateAccessorMut>(_fx: &Fixture, layer: &mut S) {
    assert_eq!(*layer.asm_recorded_epoch(), Fixture::asm_recorded_epoch());

    let epoch = EpochCommitment::new(
        Fixture::EPOCH,
        Fixture::SLOT,
        OLBlockId::from(Buf32::from([9u8; 32])),
    );
    layer.set_asm_recorded_epoch(epoch);
    assert_eq!(*layer.asm_recorded_epoch(), epoch);
}

/// [`append_l1_block_rec`](IStateAccessorMut::append_l1_block_rec) reads back via
/// [`last_l1_height`](IStateAccessor::last_l1_height) and
/// [`last_l1_blkid`](IStateAccessor::last_l1_blkid).
pub(crate) fn roundtrip_last_l1_block_rec<S: IStateAccessorMut>(_fx: &Fixture, layer: &mut S) {
    // Indices into the MMR are L1 heights, so the next record goes at the height
    // matching the current entry count.
    let entries_before = layer.l1_block_refs_mmr().num_entries();
    let height = L1Height::from(entries_before as u32);
    let block_hash = [7u8; 32];
    layer.append_l1_block_rec(height, L1BlockRecord::new(block_hash, [8u8; 32]));

    assert_eq!(layer.last_l1_height(), height);
    assert_eq!(
        *layer.last_l1_blkid(),
        L1BlockId::from(Buf32::from(block_hash))
    );

    // The append grows the underlying MMR.
    assert_eq!(layer.l1_block_refs_mmr().num_entries(), entries_before + 1);
}

/// Inserting an inbox message is visible through the layer but leaves the base
/// intact.
pub(crate) fn insert_inbox_message_isolated_from_base<S: IStateAccessorMut>(
    fx: &Fixture,
    layer: &mut S,
) {
    let msg = test_message_entry(50, 0, 2_000);
    layer
        .update_account(fx.account_id(), |acct| {
            acct.as_snark_account_mut()
                .unwrap()
                .insert_inbox_message(msg)
        })
        .unwrap()
        .unwrap();

    let account = layer.get_account_state(fx.account_id()).unwrap().unwrap();
    assert_eq!(
        account
            .as_snark_account()
            .unwrap()
            .inbox_mmr()
            .num_entries(),
        1
    );

    // Base is untouched.
    let base_account = fx
        .base()
        .get_account_state(fx.account_id())
        .unwrap()
        .unwrap();
    assert_eq!(
        base_account
            .as_snark_account()
            .unwrap()
            .inbox_mmr()
            .num_entries(),
        0
    );
}

/// A write changes the computed state root relative to the pre-write root.
pub(crate) fn state_root_changes_after_write<S: IStateAccessorMut>(_fx: &Fixture, layer: &mut S) {
    let before = layer.compute_state_root().expect("state root before write");

    layer.set_cur_slot(Fixture::SLOT + 42);

    let after = layer.compute_state_root().expect("state root after write");
    assert_ne!(before, after);
}

/// Updating a nonexistent account returns [`StateError::MissingAccount`].
pub(crate) fn update_nonexistent_account_errors<S: IStateAccessorMut>(
    _fx: &Fixture,
    layer: &mut S,
) {
    let result = layer.update_account(Fixture::unused_account_id(0), |_acct| {});
    assert!(matches!(result, Err(StateError::MissingAccount(_))));
}

/// Appending a pending ASM log stacks on top of the base's entries and is
/// visible through the layer.
pub(crate) fn pending_asm_log_append_visible<S: IStateAccessorMut>(_fx: &Fixture, layer: &mut S) {
    assert_eq!(layer.pending_asm_logs_len(), Fixture::PENDING_LOGS);
    layer
        .try_append_pending_asm_log(test_pending_asm_log(42))
        .expect("append");

    assert_eq!(layer.pending_asm_logs_len(), Fixture::PENDING_LOGS + 1);
    let heights: Vec<L1Height> = (0..Fixture::PENDING_LOGS + 1)
        .map(|i| layer.get_pending_asm_log(i).unwrap().height())
        .collect();

    // The fixture's entries carry their index as their height.
    let expected: Vec<L1Height> = (0..Fixture::PENDING_LOGS as L1Height)
        .chain(iter::once(42))
        .collect();
    assert_eq!(heights, expected);
    assert!(
        layer
            .get_pending_asm_log(Fixture::PENDING_LOGS + 1)
            .is_none()
    );
}

/// Resetting intraepoch state hides the base's pending ASM logs without
/// mutating the base.
pub(crate) fn reset_hides_base_pending_logs<S: IStateAccessorMut>(fx: &Fixture, layer: &mut S) {
    assert_eq!(layer.pending_asm_logs_len(), Fixture::PENDING_LOGS);
    layer.reset_intraepoch_state();
    assert_eq!(layer.pending_asm_logs_len(), 0);

    layer
        .try_append_pending_asm_log(test_pending_asm_log(7))
        .expect("append after reset");
    assert_eq!(layer.pending_asm_logs_len(), 1);
    assert_eq!(
        layer.get_pending_asm_log(0).unwrap().height(),
        L1Height::from(7u32)
    );

    // Base entries remain untouched.
    assert_eq!(fx.base().pending_asm_logs_len(), Fixture::PENDING_LOGS);
}

/// Instantiates the shared read-behavior tests for a stack-builder macro.
///
/// `$build` names a macro taking `($base:expr, $layer:ident)` that expands to
/// the `let` chain building the stack. It must be in scope at the invocation
/// site.
macro_rules! impl_read_layer_tests {
    ($build:ident) => {
        $crate::common_tests::read_layer_test!(
            $build,
            common_read_falls_back_to_base,
            read_falls_back_to_base
        );
        $crate::common_tests::read_layer_test!(
            $build,
            common_check_account_exists_falls_back_to_base,
            check_account_exists_falls_back_to_base
        );
        $crate::common_tests::read_layer_test!(
            $build,
            common_reads_all_fields_from_base,
            reads_all_fields_from_base
        );
        $crate::common_tests::read_layer_test!(
            $build,
            common_find_account_by_serial_from_base,
            find_account_by_serial_from_base
        );
        $crate::common_tests::read_layer_test!(
            $build,
            common_find_serial_returns_none_for_unknown,
            find_serial_returns_none_for_unknown
        );
        $crate::common_tests::read_layer_test!(
            $build,
            common_get_missing_account_returns_none,
            get_missing_account_returns_none
        );
        $crate::common_tests::read_layer_test!(
            $build,
            common_state_root_matches_base_with_no_writes,
            state_root_matches_base_with_no_writes
        );
    };
}

/// Generates one read-behavior test over the stack built by `$build`.
macro_rules! read_layer_test {
    ($build:ident, $test_name:ident, $behavior:ident) => {
        #[test]
        fn $test_name() {
            let fx = $crate::common_tests::Fixture::new();
            $build!(fx.base(), layer);
            $crate::common_tests::$behavior(&fx, &layer);
        }
    };
}

/// Instantiates the shared write-behavior tests for a stack-builder macro.
///
/// `$build` names a macro taking `($base:expr, mut $layer:ident)` that expands
/// to the `let` chain building the stack. It must be in scope at the invocation
/// site.
macro_rules! impl_mut_layer_tests {
    ($build:ident) => {
        $crate::common_tests::mut_layer_test!(
            $build,
            common_update_account_isolated_from_base,
            update_account_isolated_from_base
        );
        $crate::common_tests::mut_layer_test!(
            $build,
            common_repeated_update_accumulates,
            repeated_update_accumulates
        );
        $crate::common_tests::mut_layer_test!(
            $build,
            common_create_account_visible,
            create_account_visible
        );
        $crate::common_tests::mut_layer_test!(
            $build,
            common_create_empty_account,
            create_empty_account
        );
        $crate::common_tests::mut_layer_test!(
            $build,
            common_create_then_update_account,
            create_then_update_account
        );
        $crate::common_tests::mut_layer_test!(
            $build,
            common_next_serial_advances_across_creates,
            next_serial_advances_across_creates
        );
        $crate::common_tests::mut_layer_test!(
            $build,
            common_create_duplicate_account_errors,
            create_duplicate_account_errors
        );
        $crate::common_tests::mut_layer_test!(
            $build,
            common_create_duplicate_of_base_account_errors,
            create_duplicate_of_base_account_errors
        );
        $crate::common_tests::mut_layer_test!(
            $build,
            common_set_proof_state_roundtrip,
            set_proof_state_roundtrip
        );
        $crate::common_tests::mut_layer_test!(
            $build,
            common_take_balance_reads_back,
            take_balance_reads_back
        );
        $crate::common_tests::mut_layer_test!(
            $build,
            common_take_balance_exact_empties_account,
            take_balance_exact_empties_account
        );
        $crate::common_tests::mut_layer_test!(
            $build,
            common_take_balance_insufficient_errors,
            take_balance_insufficient_errors
        );
        $crate::common_tests::mut_layer_test!(
            $build,
            common_take_limbo_funds_insufficient_errors,
            take_limbo_funds_insufficient_errors
        );
        $crate::common_tests::mut_layer_test!(
            $build,
            common_add_limbo_funds_overflow_errors,
            add_limbo_funds_overflow_errors
        );
        $crate::common_tests::mut_layer_test!(
            $build,
            common_insert_inbox_message_isolated_from_base,
            insert_inbox_message_isolated_from_base
        );
        $crate::common_tests::mut_layer_test!(
            $build,
            common_roundtrip_cur_slot,
            roundtrip_cur_slot
        );
        $crate::common_tests::mut_layer_test!(
            $build,
            common_roundtrip_cur_epoch,
            roundtrip_cur_epoch
        );
        $crate::common_tests::mut_layer_test!(
            $build,
            common_roundtrip_limbo_funds,
            roundtrip_limbo_funds
        );
        $crate::common_tests::mut_layer_test!(
            $build,
            common_roundtrip_total_ledger_balance,
            roundtrip_total_ledger_balance
        );
        $crate::common_tests::mut_layer_test!(
            $build,
            common_roundtrip_asm_recorded_epoch,
            roundtrip_asm_recorded_epoch
        );
        $crate::common_tests::mut_layer_test!(
            $build,
            common_roundtrip_last_l1_block_rec,
            roundtrip_last_l1_block_rec
        );
        $crate::common_tests::mut_layer_test!(
            $build,
            common_state_root_changes_after_write,
            state_root_changes_after_write
        );
        $crate::common_tests::mut_layer_test!(
            $build,
            common_update_nonexistent_account_errors,
            update_nonexistent_account_errors
        );
        $crate::common_tests::mut_layer_test!(
            $build,
            common_pending_asm_log_append_visible,
            pending_asm_log_append_visible
        );
        $crate::common_tests::mut_layer_test!(
            $build,
            common_reset_hides_base_pending_logs,
            reset_hides_base_pending_logs
        );
    };
}

/// Generates one write-behavior test over the stack built by `$build`.
macro_rules! mut_layer_test {
    ($build:ident, $test_name:ident, $behavior:ident) => {
        #[test]
        fn $test_name() {
            let fx = $crate::common_tests::Fixture::new();
            $build!(fx.base(), mut layer);
            $crate::common_tests::$behavior(&fx, &mut layer);
        }
    };
}

pub(crate) use impl_mut_layer_tests;
pub(crate) use impl_read_layer_tests;
pub(crate) use mut_layer_test;
pub(crate) use read_layer_test;
