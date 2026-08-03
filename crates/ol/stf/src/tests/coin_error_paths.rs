//! Tests covering the error paths that hold a live [`Coin`] / `MsgPayloadCoin`.
//!
//! A recent refactor made OL STF value handling carry funds as a linear
//! [`Coin`], which panics on `Drop`.  Several fallible `?` early-returns used to
//! drop a still-live coin, turning an expected [`ExecError`] into a node panic.
//! These tests drive each such path and assert a clean `Err` is returned.
//!
//! Because a leaked coin panics on drop, every one of these tests *also*
//! implicitly proves no coin was lost: if the fix regressed, the test would
//! abort with `coin: accidentally destroyed value` rather than fail an assert.

use strata_acct_types::{BRIDGE_GATEWAY_ACCT_ID, BitcoinAmount, MsgPayloadData, TxEffects};
use strata_ledger_types::{Coin, IStateAccessorMut, StateError};

use crate::{
    account_processing,
    context::{BasicExecContext, BlockInfo},
    errors::ExecError,
    msg_payload_coin::MsgPayloadCoin,
    output::ExecOutputBuffer,
    test_utils::*,
    transaction_processing,
};

/// T1: `credit_account` recovers and defuses the coin when `update_account`
/// errors before the closure runs (the target account does not exist).
#[test]
fn credit_account_missing_account_returns_clean_error() {
    let mut state = make_genesis_state();
    let nonexistent = make_account_id(TEST_NONEXISTENT_ID);
    let value = BitcoinAmount::from_sat(3_000_000);

    let err = account_processing::credit_account_noop(
        &mut state,
        nonexistent,
        Coin::new_unchecked(value),
    )
    .expect_err("crediting a nonexistent account should error, not panic");

    assert!(
        matches!(err, ExecError::State(StateError::MissingAccount(id)) if id == nonexistent),
        "expected MissingAccount, got {err:?}",
    );
}

/// T2: `handle_misplaced_funds` (via the limbo leaf impl) defuses the coin when
/// the limbo balance would overflow, instead of dropping it live.
#[test]
fn limbo_overflow_returns_clean_error() {
    let mut state = make_genesis_state();

    // Fill limbo to just below the u64 sat ceiling so a further deposit overflows.
    state
        .add_limbo_funds_coin(Coin::new_unchecked(BitcoinAmount::from_sat(u64::MAX - 100)))
        .expect("seeding limbo near the ceiling should succeed");

    let err = account_processing::handle_misplaced_funds(
        &mut state,
        Coin::new_unchecked(BitcoinAmount::from_sat(200)),
    )
    .expect_err("overflowing limbo should error, not panic");

    assert!(
        matches!(err, ExecError::State(StateError::LimboFundsOverflow { .. })),
        "expected LimboFundsOverflow, got {err:?}",
    );
}

/// T3: a valid bridge withdrawal whose intent log would exceed the block log cap
/// defuses the payload on the `emit_typed_log` error instead of dropping it live.
#[test]
fn bridge_withdrawal_log_overflow_returns_clean_error() {
    use strata_bridge_params::BridgeParams;
    use strata_ol_chain_types::{MAX_LOGS_PER_BLOCK, OLLog};

    let mut state = make_genesis_state();
    let sender = make_account_id(TEST_SNARK_ACCOUNT_ID);

    // A valid, denomination-multiple withdrawal to a well-formed descriptor so
    // execution reaches the `emit_typed_log` call.
    let withdrawal_amount = BitcoinAmount::from_sat(100_000_000);
    let dest_desc = make_p2wpkh_bosd_descriptor(0x14);
    let msg_bytes = make_withdrawal_payload(dest_desc);
    let msg_data: MsgPayloadData = msg_bytes
        .try_into()
        .expect("withdrawal message payload should fit within SSZ max length");
    let payload = MsgPayloadCoin::new(Coin::new_unchecked(withdrawal_amount), msg_data);

    // Pre-fill the output buffer to the log cap so the withdrawal intent log
    // pushes it over.
    let outputs = ExecOutputBuffer::new_empty();
    outputs
        .emit_logs((0..MAX_LOGS_PER_BLOCK).map(|i| OLLog::new((i as u32).into(), vec![])))
        .expect("filling logs to the cap should succeed");

    let block_info = BlockInfo::new(1, 1, 1);
    let context =
        BasicExecContext::new(block_info, &outputs).with_bridge_params(BridgeParams::default());

    let err = account_processing::process_message(
        &mut state,
        sender,
        BRIDGE_GATEWAY_ACCT_ID,
        payload,
        &context,
    )
    .expect_err("a log-cap overflow on a valid withdrawal should error, not panic");

    assert!(
        matches!(err, ExecError::LogsOverflow { .. }),
        "expected LogsOverflow, got {err:?}",
    );
}

/// T4: `apply_tx_effects` defuses the still-undistributed `remaining` coin when a
/// per-effect delivery errors mid-loop.
#[test]
fn apply_tx_effects_defuses_remaining_on_midloop_error() {
    let mut state = make_genesis_state();
    let source = make_account_id(TEST_SNARK_ACCOUNT_ID);
    setup_genesis_with_snark_account(&mut state, source, 100_000_000);
    let nonexistent = make_account_id(TEST_NONEXISTENT_ID);

    // Fill limbo to the ceiling so the first transfer's sweep-to-limbo overflows.
    state
        .add_limbo_funds_coin(Coin::new_unchecked(BitcoinAmount::from_sat(u64::MAX - 100)))
        .expect("seeding limbo near the ceiling should succeed");

    // Two transfers to a nonexistent account: the first sweeps to limbo and
    // overflows, so the loop errors while `remaining` still holds the second
    // effect's value.
    let mut effects = TxEffects::default();
    assert!(effects.push_transfer(nonexistent, 200));
    assert!(effects.push_transfer(nonexistent, 50));

    let outputs = ExecOutputBuffer::new_empty();
    let context = BasicExecContext::new(BlockInfo::new(1, 0, 0), &outputs);

    let err = transaction_processing::apply_tx_effects(&mut state, source, &effects, &context)
        .expect_err("a mid-loop limbo overflow should error, not panic");

    assert!(
        matches!(err, ExecError::State(StateError::LimboFundsOverflow { .. })),
        "expected LimboFundsOverflow, got {err:?}",
    );
}
