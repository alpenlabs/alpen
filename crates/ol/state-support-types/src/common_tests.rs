//! Generic behavioral tests shared across the state access layers.
//!
//! The wrapper layers in this crate ([`BatchDiffState`], [`WriteTrackingState`],
//! [`IndexerState`], [`DaAccumulatingState`]) all expose the same
//! [`IStateAccessor`]/[`IStateAccessorMut`] surface over a base layer. The tests
//! that exercise only that surface behave identically regardless of which
//! wrapper is under test, so they are defined once here and instantiated per
//! layer via [`impl_read_layer_tests!`] and [`impl_mut_layer_tests!`], mirroring
//! the `db/tests` + `db/store-sled` pattern.
//!
//! Tests that assert on layer-specific internals (e.g. batch extraction,
//! captured indexer writes, DA blob encoding) stay in the individual layer
//! modules.
//!
//! [`BatchDiffState`]: crate::BatchDiffState
//! [`WriteTrackingState`]: crate::WriteTrackingState
//! [`IndexerState`]: crate::IndexerState
//! [`DaAccumulatingState`]: crate::DaAccumulatingState

use strata_acct_types::{BitcoinAmount, L1BlockRecord};
use strata_identifiers::{Buf32, EpochCommitment, L1BlockId, L1Height, OLBlockId};
use strata_ledger_types::{
    Coin, IAccountState, IAccountStateMut, ISnarkAccountState, ISnarkAccountStateMut,
    IStateAccessor, IStateAccessorMut, StateError,
};

use crate::{
    BatchDiffState, DaAccumulatingState, IndexerState, WriteTrackingState,
    memory_state_layer::MemoryStateBaseLayer, test_utils::*,
};

/// Builds the layer-under-test over a borrowed base [`MemoryStateBaseLayer`].
///
/// The associated [`Layer`](ReadLayerFactory::Layer) is a GAT so the produced
/// layer can borrow from `base`. Factories compose: leaf factories build a
/// layer directly over the base, while wrapper factories (e.g. [`IndexerWrap`],
/// [`DaAccumulatingWrap`]) are generic over the inner factory whose layer they
/// wrap. This lets a single test suite run against arbitrary layer stacks.
pub(crate) trait ReadLayerFactory {
    /// The layer under test, borrowing from the base for `'a`.
    type Layer<'a>: IStateAccessor
    where
        Self: 'a;

    /// Builds the layer over the given base.
    fn build<'a>(&self, base: &'a MemoryStateBaseLayer) -> Self::Layer<'a>;
}

/// A [`ReadLayerFactory`] whose produced layer is also mutable.
///
/// Any read factory whose [`Layer`](ReadLayerFactory::Layer) implements
/// [`IStateAccessorMut`] automatically implements this via the blanket impl
/// below, so factory types only ever implement [`ReadLayerFactory`] and the
/// mutability of a given stack is decided structurally. The separate
/// [`build_mut`](MutLayerFactory::build_mut) exists because a trait `where`
/// clause on `Self::Layer` is not an implied bound at use sites, whereas the
/// [`MutLayer`](MutLayerFactory::MutLayer) GAT bound is.
pub(crate) trait MutLayerFactory {
    /// The mutable layer under test, borrowing from the base for `'a`.
    type MutLayer<'a>: IStateAccessorMut
    where
        Self: 'a;

    /// Builds the mutable layer over the given base.
    fn build_mut<'a>(&self, base: &'a MemoryStateBaseLayer) -> Self::MutLayer<'a>;
}

impl<F> MutLayerFactory for F
where
    F: ReadLayerFactory,
    for<'a> F::Layer<'a>: IStateAccessorMut,
{
    type MutLayer<'a>
        = F::Layer<'a>
    where
        Self: 'a;

    fn build_mut<'a>(&self, base: &'a MemoryStateBaseLayer) -> Self::MutLayer<'a> {
        self.build(base)
    }
}

// =============================================================================
// Composable factories
// =============================================================================

/// Leaf factory: a [`WriteTrackingState`] with an empty batch directly over the
/// base. This is the canonical mutable leaf.
pub(crate) struct WriteTrackingLeaf;

impl ReadLayerFactory for WriteTrackingLeaf {
    type Layer<'a> = WriteTrackingState<'a, MemoryStateBaseLayer>;

    fn build<'a>(&self, base: &'a MemoryStateBaseLayer) -> Self::Layer<'a> {
        WriteTrackingState::new_empty(base)
    }
}

/// Leaf factory: a [`BatchDiffState`] with no pending batches (pure passthrough)
/// directly over the base. Read-only.
pub(crate) struct BatchDiffLeaf;

impl ReadLayerFactory for BatchDiffLeaf {
    type Layer<'a> = BatchDiffState<'a, 'a, MemoryStateBaseLayer>;

    fn build<'a>(&self, base: &'a MemoryStateBaseLayer) -> Self::Layer<'a> {
        BatchDiffState::new(base, &[])
    }
}

/// Wrapper factory: wraps the inner factory's layer in an [`IndexerState`].
pub(crate) struct IndexerWrap<Inner>(pub(crate) Inner);

impl<Inner: ReadLayerFactory> ReadLayerFactory for IndexerWrap<Inner> {
    type Layer<'a>
        = IndexerState<Inner::Layer<'a>>
    where
        Self: 'a;

    fn build<'a>(&self, base: &'a MemoryStateBaseLayer) -> Self::Layer<'a> {
        IndexerState::new(self.0.build(base))
    }
}

/// Wrapper factory: wraps the inner factory's layer in a [`DaAccumulatingState`].
pub(crate) struct DaAccumulatingWrap<Inner>(pub(crate) Inner);

impl<Inner: ReadLayerFactory> ReadLayerFactory for DaAccumulatingWrap<Inner> {
    type Layer<'a>
        = DaAccumulatingState<Inner::Layer<'a>>
    where
        Self: 'a;

    fn build<'a>(&self, base: &'a MemoryStateBaseLayer) -> Self::Layer<'a> {
        DaAccumulatingState::new(self.0.build(base))
    }
}

// =============================================================================
// Read behaviors (IStateAccessor)
// =============================================================================

/// Reading an account present only in the base falls through to it.
pub(crate) fn read_falls_back_to_base<F: ReadLayerFactory>(factory: F) {
    let account_id = test_account_id(1);
    let (base, serial) =
        setup_layer_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1000));
    let layer = factory.build(&base);

    let account = layer.get_account_state(account_id).unwrap().unwrap();
    assert_eq!(account.serial(), serial);
    assert_eq!(account.balance(), BitcoinAmount::from_sat(1000));
}

/// `check_account_exists` reflects presence in the base.
pub(crate) fn check_account_exists_falls_back_to_base<F: ReadLayerFactory>(factory: F) {
    let account_id = test_account_id(1);
    let nonexistent_id = test_account_id(99);
    let (base, _) = setup_layer_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1000));
    let layer = factory.build(&base);

    assert!(layer.check_account_exists(account_id).unwrap());
    assert!(!layer.check_account_exists(nonexistent_id).unwrap());
}

/// Global and epochal reads fall through to the base when unmodified.
pub(crate) fn reads_global_state_from_base<F: ReadLayerFactory>(factory: F) {
    let base = new_layer_at(5, 100);
    let layer = factory.build(&base);

    assert_eq!(layer.cur_slot(), 100);
    assert_eq!(layer.cur_epoch(), 5);
}

/// A base account can be resolved by its serial.
pub(crate) fn find_account_by_serial_from_base<F: ReadLayerFactory>(factory: F) {
    let account_id = test_account_id(1);
    let (base, serial) =
        setup_layer_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1000));
    let layer = factory.build(&base);

    assert_eq!(
        layer.find_account_id_by_serial(serial).unwrap(),
        Some(account_id)
    );
}

/// With no writes, the layer's state root matches the base's.
pub(crate) fn state_root_matches_base_with_no_writes<F: ReadLayerFactory>(factory: F) {
    let base = create_test_base_layer();
    let base_root = base.compute_state_root().unwrap();
    let layer = factory.build(&base);

    assert_eq!(layer.compute_state_root().unwrap(), base_root);
}

// =============================================================================
// Write behaviors (IStateAccessorMut)
// =============================================================================

/// Updating an account is visible through the layer but leaves the base intact.
pub(crate) fn update_account_isolated_from_base<F: MutLayerFactory>(factory: F) {
    let account_id = test_account_id(1);
    let (base, _) = setup_layer_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1000));
    let mut layer = factory.build_mut(&base);

    layer
        .update_account(account_id, |acct| {
            acct.add_balance(Coin::new_unchecked(BitcoinAmount::from_sat(500)));
        })
        .unwrap();

    let account = layer.get_account_state(account_id).unwrap().unwrap();
    assert_eq!(account.balance(), BitcoinAmount::from_sat(1500));

    // Base is untouched.
    let base_account = base.get_account_state(account_id).unwrap().unwrap();
    assert_eq!(base_account.balance(), BitcoinAmount::from_sat(1000));
}

/// Repeated updates to the same account accumulate on the layer's copy.
pub(crate) fn repeated_update_accumulates<F: MutLayerFactory>(factory: F) {
    let account_id = test_account_id(1);
    let (base, _) = setup_layer_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1000));
    let mut layer = factory.build_mut(&base);

    layer
        .update_account(account_id, |acct| {
            acct.add_balance(Coin::new_unchecked(BitcoinAmount::from_sat(500)));
        })
        .unwrap();
    layer
        .update_account(account_id, |acct| {
            acct.add_balance(Coin::new_unchecked(BitcoinAmount::from_sat(100)));
        })
        .unwrap();

    let account = layer.get_account_state(account_id).unwrap().unwrap();
    assert_eq!(account.balance(), BitcoinAmount::from_sat(1600));
}

/// A freshly created account is visible through the layer and resolvable by
/// serial.
pub(crate) fn create_account_visible<F: MutLayerFactory>(factory: F) {
    let base = create_test_base_layer();
    let mut layer = factory.build_mut(&base);

    let account_id = test_account_id(1);
    let new_acct =
        test_new_snark_account_data(&test_snark_account_state(1), BitcoinAmount::from_sat(5000));
    let serial = layer.create_new_account(account_id, new_acct).unwrap();

    assert!(layer.check_account_exists(account_id).unwrap());
    let account = layer.get_account_state(account_id).unwrap().unwrap();
    assert_eq!(account.serial(), serial);
    assert_eq!(account.balance(), BitcoinAmount::from_sat(5000));
    assert_eq!(
        layer.find_account_id_by_serial(serial).unwrap(),
        Some(account_id)
    );
}

// -----------------------------------------------------------------------------
// Simple state write -> read-back roundtrips
//
// One `roundtrip_*` test per simple setter on `IStateAccessorMut`: perform the
// write, then assert the matching getter reads the value back.
// -----------------------------------------------------------------------------

/// [`set_cur_slot`](IStateAccessorMut::set_cur_slot) reads back via
/// [`cur_slot`](IStateAccessor::cur_slot).
pub(crate) fn roundtrip_cur_slot<F: MutLayerFactory>(factory: F) {
    let base = create_test_base_layer();
    let mut layer = factory.build_mut(&base);

    assert_eq!(layer.cur_slot(), 0);
    layer.set_cur_slot(42);
    assert_eq!(layer.cur_slot(), 42);
}

/// [`set_cur_epoch`](IStateAccessorMut::set_cur_epoch) reads back via
/// [`cur_epoch`](IStateAccessor::cur_epoch).
pub(crate) fn roundtrip_cur_epoch<F: MutLayerFactory>(factory: F) {
    let base = create_test_base_layer();
    let mut layer = factory.build_mut(&base);

    assert_eq!(layer.cur_epoch(), 0);
    layer.set_cur_epoch(5);
    assert_eq!(layer.cur_epoch(), 5);
}

/// [`add_limbo_funds_coin`](IStateAccessorMut::add_limbo_funds_coin) and
/// [`take_limbo_funds_coin`](IStateAccessorMut::take_limbo_funds_coin) read back
/// via [`limbo_funds`](IStateAccessor::limbo_funds).
pub(crate) fn roundtrip_limbo_funds<F: MutLayerFactory>(factory: F) {
    let base = create_test_base_layer();
    let mut layer = factory.build_mut(&base);

    assert_eq!(layer.limbo_funds(), BitcoinAmount::ZERO);

    layer
        .add_limbo_funds_coin(Coin::new_unchecked(BitcoinAmount::from_sat(1_000)))
        .unwrap();
    assert_eq!(layer.limbo_funds(), BitcoinAmount::from_sat(1_000));

    let taken = layer
        .take_limbo_funds_coin(BitcoinAmount::from_sat(400))
        .unwrap();
    taken.safely_consume_unchecked();
    assert_eq!(layer.limbo_funds(), BitcoinAmount::from_sat(600));
}

/// [`set_total_ledger_balance`](IStateAccessorMut::set_total_ledger_balance)
/// reads back via [`total_ledger_balance`](IStateAccessor::total_ledger_balance).
pub(crate) fn roundtrip_total_ledger_balance<F: MutLayerFactory>(factory: F) {
    let base = create_test_base_layer();
    let mut layer = factory.build_mut(&base);

    layer.set_total_ledger_balance(BitcoinAmount::from_sat(1_000_000));
    assert_eq!(
        layer.total_ledger_balance(),
        BitcoinAmount::from_sat(1_000_000)
    );
}

/// [`set_asm_recorded_epoch`](IStateAccessorMut::set_asm_recorded_epoch) reads
/// back via [`asm_recorded_epoch`](IStateAccessor::asm_recorded_epoch).
pub(crate) fn roundtrip_asm_recorded_epoch<F: MutLayerFactory>(factory: F) {
    let base = create_test_base_layer();
    let mut layer = factory.build_mut(&base);

    let epoch = EpochCommitment::new(3, 7, OLBlockId::from(Buf32::from([9u8; 32])));
    layer.set_asm_recorded_epoch(epoch);
    assert_eq!(*layer.asm_recorded_epoch(), epoch);
}

/// [`append_l1_block_rec`](IStateAccessorMut::append_l1_block_rec) reads back via
/// [`last_l1_height`](IStateAccessor::last_l1_height) and
/// [`last_l1_blkid`](IStateAccessor::last_l1_blkid).
pub(crate) fn roundtrip_last_l1_block_rec<F: MutLayerFactory>(factory: F) {
    let base = create_test_base_layer();
    let mut layer = factory.build_mut(&base);

    let height = L1Height::from(100u32);
    let block_hash = [7u8; 32];
    layer.append_l1_block_rec(height, L1BlockRecord::new(block_hash, [8u8; 32]));

    assert_eq!(layer.last_l1_height(), height);
    assert_eq!(
        *layer.last_l1_blkid(),
        L1BlockId::from(Buf32::from(block_hash))
    );
}

/// Inserting an inbox message is visible through the layer but leaves the base
/// intact.
pub(crate) fn insert_inbox_message_isolated_from_base<F: MutLayerFactory>(factory: F) {
    let account_id = test_account_id(1);
    let (base, _) = setup_layer_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1_000));
    let mut layer = factory.build_mut(&base);

    let msg = test_message_entry(50, 0, 2_000);
    layer
        .update_account(account_id, |acct| {
            acct.as_snark_account_mut()
                .unwrap()
                .insert_inbox_message(msg)
        })
        .unwrap()
        .unwrap();

    let account = layer.get_account_state(account_id).unwrap().unwrap();
    assert_eq!(
        account
            .as_snark_account()
            .unwrap()
            .inbox_mmr()
            .num_entries(),
        1
    );

    // Base is untouched.
    let base_account = base.get_account_state(account_id).unwrap().unwrap();
    assert_eq!(
        base_account
            .as_snark_account()
            .unwrap()
            .inbox_mmr()
            .num_entries(),
        0
    );
}

/// Computing the state root after a write succeeds.
pub(crate) fn compute_state_root_with_writes_succeeds<F: MutLayerFactory>(factory: F) {
    let base = create_test_base_layer();
    let mut layer = factory.build_mut(&base);

    layer.set_cur_slot(42);

    layer
        .compute_state_root()
        .expect("state root should succeed after writes");
}

/// Updating a nonexistent account returns [`StateError::MissingAccount`].
pub(crate) fn update_nonexistent_account_errors<F: MutLayerFactory>(factory: F) {
    let base = create_test_base_layer();
    let mut layer = factory.build_mut(&base);

    let result = layer.update_account(test_account_id(99), |_acct| {});
    assert!(matches!(result, Err(StateError::MissingAccount(_))));
}

/// Appending a pending ASM log stacks on top of the base's entries and is
/// visible through the layer.
pub(crate) fn pending_asm_log_append_visible<F: MutLayerFactory>(factory: F) {
    let mut base = create_test_base_layer();
    base.try_append_pending_asm_log(test_pending_asm_log(0))
        .expect("base append");
    base.try_append_pending_asm_log(test_pending_asm_log(1))
        .expect("base append");
    let mut layer = factory.build_mut(&base);

    assert_eq!(layer.pending_asm_logs_len(), 2);
    layer
        .try_append_pending_asm_log(test_pending_asm_log(42))
        .expect("append");

    assert_eq!(layer.pending_asm_logs_len(), 3);
    let heights: Vec<L1Height> = (0..3)
        .map(|i| layer.get_pending_asm_log(i).unwrap().height())
        .collect();
    assert_eq!(
        heights,
        vec![
            L1Height::from(0u32),
            L1Height::from(1u32),
            L1Height::from(42u32),
        ]
    );
    assert!(layer.get_pending_asm_log(3).is_none());
}

/// Resetting intraepoch state hides the base's pending ASM logs without
/// mutating the base.
pub(crate) fn reset_hides_base_pending_logs<F: MutLayerFactory>(factory: F) {
    let mut base = create_test_base_layer();
    base.try_append_pending_asm_log(test_pending_asm_log(0))
        .expect("base append");
    base.try_append_pending_asm_log(test_pending_asm_log(1))
        .expect("base append");
    base.try_append_pending_asm_log(test_pending_asm_log(2))
        .expect("base append");
    let mut layer = factory.build_mut(&base);

    assert_eq!(layer.pending_asm_logs_len(), 3);
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
    assert_eq!(base.pending_asm_logs_len(), 3);
}

/// Instantiates the shared read-behavior tests for a [`ReadLayerFactory`].
macro_rules! impl_read_layer_tests {
    ($factory:expr) => {
        #[test]
        fn common_read_falls_back_to_base() {
            $crate::common_tests::read_falls_back_to_base($factory);
        }

        #[test]
        fn common_check_account_exists_falls_back_to_base() {
            $crate::common_tests::check_account_exists_falls_back_to_base($factory);
        }

        #[test]
        fn common_reads_global_state_from_base() {
            $crate::common_tests::reads_global_state_from_base($factory);
        }

        #[test]
        fn common_find_account_by_serial_from_base() {
            $crate::common_tests::find_account_by_serial_from_base($factory);
        }

        #[test]
        fn common_state_root_matches_base_with_no_writes() {
            $crate::common_tests::state_root_matches_base_with_no_writes($factory);
        }
    };
}

/// Instantiates the shared write-behavior tests for a [`MutLayerFactory`].
macro_rules! impl_mut_layer_tests {
    ($factory:expr) => {
        #[test]
        fn common_update_account_isolated_from_base() {
            $crate::common_tests::update_account_isolated_from_base($factory);
        }

        #[test]
        fn common_repeated_update_accumulates() {
            $crate::common_tests::repeated_update_accumulates($factory);
        }

        #[test]
        fn common_create_account_visible() {
            $crate::common_tests::create_account_visible($factory);
        }

        #[test]
        fn common_insert_inbox_message_isolated_from_base() {
            $crate::common_tests::insert_inbox_message_isolated_from_base($factory);
        }

        #[test]
        fn common_roundtrip_cur_slot() {
            $crate::common_tests::roundtrip_cur_slot($factory);
        }

        #[test]
        fn common_roundtrip_cur_epoch() {
            $crate::common_tests::roundtrip_cur_epoch($factory);
        }

        #[test]
        fn common_roundtrip_limbo_funds() {
            $crate::common_tests::roundtrip_limbo_funds($factory);
        }

        #[test]
        fn common_roundtrip_total_ledger_balance() {
            $crate::common_tests::roundtrip_total_ledger_balance($factory);
        }

        #[test]
        fn common_roundtrip_asm_recorded_epoch() {
            $crate::common_tests::roundtrip_asm_recorded_epoch($factory);
        }

        #[test]
        fn common_roundtrip_last_l1_block_rec() {
            $crate::common_tests::roundtrip_last_l1_block_rec($factory);
        }

        #[test]
        fn common_compute_state_root_with_writes_succeeds() {
            $crate::common_tests::compute_state_root_with_writes_succeeds($factory);
        }

        #[test]
        fn common_update_nonexistent_account_errors() {
            $crate::common_tests::update_nonexistent_account_errors($factory);
        }

        #[test]
        fn common_pending_asm_log_append_visible() {
            $crate::common_tests::pending_asm_log_append_visible($factory);
        }

        #[test]
        fn common_reset_hides_base_pending_logs() {
            $crate::common_tests::reset_hides_base_pending_logs($factory);
        }
    };
}

pub(crate) use impl_mut_layer_tests;
pub(crate) use impl_read_layer_tests;
