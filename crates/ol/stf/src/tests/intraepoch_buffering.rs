//! Tests for intraepoch ASM-log buffering and epoch-terminal draining.
//!
//! Manifests may be included in any block within an epoch; their ASM logs are
//! buffered into intraepoch state and their effects are applied only at the
//! epoch terminal (signalled by the `IS_TERMINAL` header flag), after which the
//! intraepoch state is reset and the epoch advances.

use strata_acct_types::BitcoinAmount;
use strata_asm_common::AsmLogEntry;
use strata_identifiers::SubjectId;
use strata_ledger_types::{
    ISnarkAccountState, IStateAccessor, IStateAccessorMut, PendingAsmLog, StateError,
};

use crate::{buffer_block_manifests, errors::ExecError, test_utils::*};

/// A manifest in a non-terminal block buffers its logs (and eagerly advances
/// the ASM MMR / `last_l1_height`) without applying the deposit effect; the
/// effect materializes only when a later terminal block drains the buffer.
#[test]
fn manifest_in_non_terminal_block_buffers_then_drains_at_terminal() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let dest_subject = SubjectId::from([42u8; 32]);
    let deposit_amount = BitcoinAmount::from_sat(150_000_000);

    let fixture_builder = OLStfFixture::builder();
    let snark_acct_serial = fixture_builder.next_account_serial();
    let mut fixture = fixture_builder
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(0))
                .with_state_root(make_state_root(1))
        })
        .execute_genesis();

    // Genesis carried no manifests, so the running L1 cursor is still 0.
    assert_eq!(fixture.state().last_l1_height(), 0);
    let mmr_before = fixture.state().l1_block_refs_mmr().num_entries();

    // Non-terminal block carrying a deposit manifest at height 1.
    let deposit_manifest =
        make_deposit_manifest_for_account(1, 1, snark_acct_serial, dest_subject, deposit_amount);
    fixture
        .child_block()
        .with_manifest(deposit_manifest)
        .execute();

    // The log is buffered, not applied: balance/inbox unchanged, but the MMR
    // and L1 cursor advanced eagerly and the epoch has not advanced.
    assert_eq!(
        fixture.state().pending_asm_logs_len(),
        1,
        "deposit log should be buffered in intraepoch state"
    );
    assert_eq!(
        fixture.account_balance(snark_acct_id),
        BitcoinAmount::from_sat(0),
        "deposit effect must not be applied before the terminal"
    );
    assert_eq!(
        fixture
            .expect_snark_account(snark_acct_id)
            .inbox_mmr()
            .num_entries(),
        0,
        "no inbox message before the terminal drain"
    );
    assert_eq!(
        fixture.state().last_l1_height(),
        1,
        "MMR cursor advances eagerly"
    );
    assert_eq!(
        fixture.state().l1_block_refs_mmr().num_entries(),
        mmr_before + 1,
        "manifest appended to MMR eagerly"
    );
    assert_eq!(
        fixture.state().cur_epoch(),
        1,
        "epoch must not advance pre-terminal"
    );

    // Terminal block (no new manifests) drains the buffer and applies effects.
    fixture.child_block().terminal().execute();

    assert_eq!(
        fixture.state().pending_asm_logs_len(),
        0,
        "intraepoch buffer must be reset after the terminal"
    );
    assert_eq!(
        fixture.account_balance(snark_acct_id),
        deposit_amount,
        "deposit effect should be applied at the terminal drain"
    );
    assert_eq!(
        fixture
            .expect_snark_account(snark_acct_id)
            .inbox_mmr()
            .num_entries(),
        1,
        "deposit message delivered to inbox at the terminal drain"
    );
    assert_eq!(
        fixture.state().cur_epoch(),
        2,
        "epoch advances at the terminal"
    );
}

/// Manifests spread across several non-terminal blocks in an epoch all buffer
/// (with cross-block height continuity), then drain together at the terminal.
#[test]
fn multi_block_epoch_manifests_spread_across_blocks() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let dest_subject = SubjectId::from([7u8; 32]);
    let amount = BitcoinAmount::from_sat(50_000_000);

    let fixture_builder = OLStfFixture::builder();
    let serial = fixture_builder.next_account_serial();
    let mut fixture = fixture_builder
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(0))
                .with_state_root(make_state_root(1))
        })
        .execute_genesis();

    let mmr_before = fixture.state().l1_block_refs_mmr().num_entries();

    // Block A (non-terminal): manifest at height 1.
    fixture
        .child_block()
        .with_manifest(make_deposit_manifest_for_account(
            1,
            1,
            serial,
            dest_subject,
            amount,
        ))
        .execute();
    // Block B (non-terminal): manifest at height 2.
    fixture
        .child_block()
        .with_manifest(make_deposit_manifest_for_account(
            2,
            2,
            serial,
            dest_subject,
            amount,
        ))
        .execute();

    assert_eq!(fixture.state().pending_asm_logs_len(), 2);
    assert_eq!(
        fixture.account_balance(snark_acct_id),
        BitcoinAmount::from_sat(0)
    );
    assert_eq!(fixture.state().last_l1_height(), 2);

    // Terminal block C: manifest at height 3, then drain all three deposits.
    fixture
        .child_block()
        .with_manifest(make_deposit_manifest_for_account(
            3,
            3,
            serial,
            dest_subject,
            amount,
        ))
        .terminal()
        .execute();

    assert_eq!(fixture.state().pending_asm_logs_len(), 0);
    assert_eq!(
        fixture.account_balance(snark_acct_id),
        BitcoinAmount::from_sat(150_000_000),
        "all three buffered deposits applied at the terminal"
    );
    assert_eq!(
        fixture
            .expect_snark_account(snark_acct_id)
            .inbox_mmr()
            .num_entries(),
        3,
        "all three deposit messages delivered"
    );
    assert_eq!(
        fixture.state().l1_block_refs_mmr().num_entries(),
        mmr_before + 3,
        "all three manifests appended to the MMR"
    );
    assert_eq!(fixture.state().cur_epoch(), 2);
}

/// Manifest heights must stay strictly sequential across blocks; a gap is
/// rejected even when the manifests arrive in separate blocks.
#[test]
fn cross_block_manifest_height_gap_is_rejected() {
    let mut fixture = OLStfFixture::builder().execute_genesis();

    // Block A buffers height 1.
    fixture
        .child_block()
        .with_manifest(make_empty_manifest(1, 1))
        .execute();

    // Block B tries height 3 (skipping 2) -> rejected.
    let err = fixture
        .child_block()
        .with_manifest(make_empty_manifest(3, 2))
        .execute_err();

    match err.into_base() {
        ExecError::AsmManifestHeightMismatch {
            expected,
            actual,
            index,
        } => assert_eq!((expected, actual, index), (2, 3, 0)),
        other => panic!("expected AsmManifestHeightMismatch, got {other:?}"),
    }
}

/// A terminal block with an empty intraepoch buffer (no manifests anywhere in
/// the epoch) still resets and advances the epoch cleanly.
#[test]
fn terminal_with_empty_buffer_advances_epoch() {
    let mut fixture = OLStfFixture::builder().execute_genesis();
    assert_eq!(fixture.state().cur_epoch(), 1);

    // A non-terminal block with no manifests, then a terminal with empty buffer.
    fixture.child_block().execute();
    fixture.child_block().terminal().execute();

    assert_eq!(fixture.state().pending_asm_logs_len(), 0);
    assert_eq!(fixture.state().cur_epoch(), 2);
}

/// `buffer_block_manifests` surfaces [`StateError::PendingAsmLogsFull`] (wrapped
/// in [`ExecError::State`]) once the intraepoch buffer reaches capacity.
#[test]
fn intraepoch_buffer_full_is_rejected() {
    let mut state = make_genesis_state();

    // Fill the buffer to capacity directly via the accessor.
    let raw_log = AsmLogEntry::from_raw(vec![0xff]).expect("raw log should fit");
    let mut appended = 0u64;
    while state
        .try_append_pending_asm_log(PendingAsmLog::new(1, raw_log.clone()))
        .is_ok()
    {
        appended += 1;
    }
    assert!(
        state.pending_asm_logs_full(),
        "buffer should be at capacity"
    );
    assert!(appended > 0);

    // Buffering a further manifest log must surface the full-buffer error.
    let manifest = FixtureAsmManifestBuilder::new_at_height(1)
        .with_log(raw_log)
        .build();
    let err = buffer_block_manifests(&mut state, &[manifest])
        .expect_err("buffering into a full intraepoch buffer must error");

    assert!(
        matches!(
            err.into_base(),
            ExecError::State(StateError::PendingAsmLogsFull)
        ),
        "expected PendingAsmLogsFull"
    );
}
