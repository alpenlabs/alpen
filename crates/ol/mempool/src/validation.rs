//! Transaction validation for mempool using STF helpers.
use std::collections::HashMap;

use strata_acct_types::{AccountId, AcctError};
use strata_identifiers::OLTxId;
use strata_ledger_types::{IAccountState, IStateAccessor, IStateAccessorMut};
use strata_ol_chain_types_new::{OLTransaction, TransactionPayload};
use strata_ol_stf::{ExecError, ExecResult, check_tx_constraints};
use strata_snark_acct_sys as snark_sys;
use strata_snark_acct_types::Seqno;
use tracing::error;

use crate::{OLMempoolError, OLMempoolResult, state::AccountMempoolState};

/// Checks sequence number against a range and returns appropriate error.
/// - `tx_seq_no < min_expected` → `UsedSequenceNumber`
/// - `tx_seq_no > max_expected` → `SequenceNumberGap`
fn check_seq_no_in_range(
    txid: OLTxId,
    tx_seq_no: u64,
    min_expected: u64,
    max_expected: u64,
) -> OLMempoolResult<()> {
    if tx_seq_no < min_expected {
        return Err(OLMempoolError::UsedSequenceNumber {
            txid,
            expected: min_expected,
            actual: tx_seq_no,
        });
    }
    if tx_seq_no > max_expected {
        return Err(OLMempoolError::SequenceNumberGap {
            expected: max_expected,
            actual: tx_seq_no,
        });
    }
    Ok(())
}

/// Converts an [`AcctError::InvalidUpdateSequence`] to the appropriate mempool error.
/// - `got < expected` → `UsedSequenceNumber`
/// - `got > expected` → `SequenceNumberGap`
fn seq_no_error_to_mempool_error(txid: OLTxId, expected: u64, got: u64) -> OLMempoolError {
    if got < expected {
        OLMempoolError::UsedSequenceNumber {
            txid,
            expected,
            actual: got,
        }
    } else {
        OLMempoolError::SequenceNumberGap {
            expected,
            actual: got,
        }
    }
}

/// Validates sequence number for a
/// [`SnarkAccountUpdate`](strata_snark_acct_types::SnarkAccountUpdate) transaction.
///
/// Checks sequence number against either:
/// - Mempool state (if there are pending
///   [`SnarkAccountUpdate`](strata_snark_acct_types::SnarkAccountUpdate) transactions)
/// - On-chain state (if no pending transactions)
fn validate_snark_account_update_tx_seq_no(
    txid: OLTxId,
    target_account: AccountId,
    tx_seq_no: u64,
    mempool_seq_no_range: Option<(u64, u64)>,
    state_accessor: &impl IStateAccessor,
) -> OLMempoolResult<()> {
    if let Some((min_seq_no, max_seq_no)) = mempool_seq_no_range {
        // Has SnarkAccountUpdate transactions in mempool - validate against range
        check_seq_no_in_range(txid, tx_seq_no, min_seq_no, max_seq_no + 1)?;
    } else {
        // No SnarkAccountUpdate transactions in mempool - validate against on-chain state
        let account_state =
            get_account_state(state_accessor, target_account).map_err(|e| match e {
                ExecError::UnknownAccount(account) => {
                    error!(
                        %txid,
                        ?account,
                        "account disappeared between existence check and seq_no retrieval"
                    );
                    OLMempoolError::AccountDoesNotExist { account }
                }
                _ => {
                    OLMempoolError::AccountStateAccess(format!("Failed to get account state: {e}"))
                }
            })?;

        let snark_state =
            account_state
                .as_snark_account()
                .map_err(|_| OLMempoolError::AccountTypeMismatch {
                    txid,
                    account: target_account,
                })?;

        let res = snark_sys::verify_seq_no(target_account, snark_state, Seqno::from(tx_seq_no));
        if let Err(ExecError::Acct(AcctError::InvalidUpdateSequence { expected, got, .. })) = res {
            return Err(seq_no_error_to_mempool_error(txid, expected, got));
        }
    }

    Ok(())
}

/// Validates a transaction using STF validation helpers.
///
/// Performs stateful validation:
/// - Slot bounds checking
/// - Account existence checking
/// - Sequence number validation (for
///   [`SnarkAccountUpdate`](strata_snark_acct_types::SnarkAccountUpdate) transactions)
pub(crate) fn validate_transaction(
    txid: OLTxId,
    tx: &OLTransaction,
    state_accessor: &impl IStateAccessorMut,
    account_state: &HashMap<AccountId, AccountMempoolState>,
) -> OLMempoolResult<()> {
    let target_account = tx
        .target()
        .expect("all OL payload variants must have a target");

    // 1. Slot bounds check.
    check_tx_constraints(tx.constraints(), state_accessor).map_err(|e| match e {
        ExecError::TransactionExpired(max_slot, current_slot) => {
            OLMempoolError::TransactionExpired {
                txid,
                max_slot,
                current_slot,
            }
        }
        ExecError::TransactionNotMature(min_slot, current_slot) => {
            OLMempoolError::TransactionNotMature {
                txid,
                min_slot,
                current_slot,
            }
        }
        _ => OLMempoolError::AccountStateAccess(format!("Slot bounds check failed: {e}")),
    })?;

    // 2. Account existence check.
    get_account_state(state_accessor, target_account).map_err(|e| match e {
        ExecError::UnknownAccount(_) => OLMempoolError::AccountDoesNotExist {
            account: target_account,
        },
        _ => OLMempoolError::AccountStateAccess(format!("Failed to check account existence: {e}")),
    })?;

    // 3. Sequence number in proper range (for SnarkAccountUpdate transactions).
    if let TransactionPayload::SnarkAccountUpdate(payload) = tx.payload() {
        let tx_seq_no = payload.operation().update().seq_no();

        // Check if there are SnarkAccountUpdate transactions in mempool for this account
        let mempool_seq_no_range = account_state
            .get(&target_account)
            .and_then(|acct_state| acct_state.seq_no_range());

        validate_snark_account_update_tx_seq_no(
            txid,
            target_account,
            tx_seq_no,
            mempool_seq_no_range,
            state_accessor,
        )?;
    }

    Ok(())
}

/// Gets an account state, returning an error if it doesn't exist.
fn get_account_state<S: IStateAccessor>(
    state: &S,
    account: AccountId,
) -> ExecResult<&S::AccountState> {
    state
        .get_account_state(account)?
        .ok_or(ExecError::UnknownAccount(account))
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashMap};

    use super::*;
    use crate::test_utils::*;

    #[test]
    fn test_slot_bounds_expired() {
        let tx = create_test_generic_tx_with_slots(None, Some(50)); // max_slot < current_slot (100)
        let state_accessor = create_test_ol_state_with_account(tx_target(&tx), 100);
        let account_state = HashMap::new();

        let result = validate_transaction(tx.compute_txid(), &tx, &state_accessor, &account_state);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            OLMempoolError::TransactionExpired { .. }
        ));
    }

    #[test]
    fn test_slot_bounds_not_yet_valid() {
        let tx = create_test_generic_tx_with_slots(Some(150), None); // min_slot > current_slot (100)
        let state_accessor = create_test_ol_state_with_account(tx_target(&tx), 100);
        let account_state = HashMap::new();

        let result = validate_transaction(tx.compute_txid(), &tx, &state_accessor, &account_state);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            OLMempoolError::TransactionNotMature { .. }
        ));
    }

    #[test]
    fn test_min_slot_boundary() {
        // min_slot == current_slot should be valid
        let tx = create_test_generic_tx_with_slots(Some(100), None);
        let state_accessor = create_test_ol_state_with_account(tx_target(&tx), 100);
        let account_state = HashMap::new();

        let result = validate_transaction(tx.compute_txid(), &tx, &state_accessor, &account_state);
        assert!(result.is_ok());
    }

    #[test]
    fn test_max_slot_boundary() {
        // max_slot == current_slot should be valid (not expired yet)
        let tx = create_test_generic_tx_with_slots(None, Some(100));
        let state_accessor = create_test_ol_state_with_account(tx_target(&tx), 100);
        let account_state = HashMap::new();

        let result = validate_transaction(tx.compute_txid(), &tx, &state_accessor, &account_state);
        assert!(result.is_ok());
    }

    #[test]
    fn test_max_slot_one_after_current() {
        // max_slot == current_slot + 1 should be valid
        let tx = create_test_generic_tx_with_slots(None, Some(101));
        let state_accessor = create_test_ol_state_with_account(tx_target(&tx), 100);
        let account_state = HashMap::new();

        let result = validate_transaction(tx.compute_txid(), &tx, &state_accessor, &account_state);
        assert!(result.is_ok());
    }

    #[test]
    fn test_slot_bounds_valid() {
        let tx = create_test_generic_tx_with_slots(Some(50), Some(150)); // min < current < max
        let state_accessor = create_test_ol_state_with_account(tx_target(&tx), 100);
        let account_state = HashMap::new();

        let result = validate_transaction(tx.compute_txid(), &tx, &state_accessor, &account_state);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validation_with_valid_transaction() {
        // Test that validation works with a normal valid transaction
        let tx = create_test_generic_tx_with_slots(Some(50), Some(150));
        let state_accessor = create_test_ol_state_with_account(tx_target(&tx), 100);
        let account_state = HashMap::new();

        let result = validate_transaction(tx.compute_txid(), &tx, &state_accessor, &account_state);
        assert!(result.is_ok());
    }

    #[test]
    fn test_slot_bounds_no_constraints() {
        let tx = create_test_generic_tx_with_slots(None, None); // No slot bounds
        let state_accessor = create_test_ol_state_with_account(tx_target(&tx), 100);
        let account_state = HashMap::new();

        let result = validate_transaction(tx.compute_txid(), &tx, &state_accessor, &account_state);
        assert!(result.is_ok());
    }

    #[test]
    fn test_account_does_not_exist() {
        let tx = create_test_generic_tx_with_slots(Some(50), Some(150));
        // Don't create account in state - should fail existence check
        let state_accessor = create_test_ol_state_with_account(create_test_account_id(), 100); // Different account
        let account_state = HashMap::new();

        let result = validate_transaction(tx.compute_txid(), &tx, &state_accessor, &account_state);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            OLMempoolError::AccountDoesNotExist { .. }
        ));
    }

    #[test]
    fn test_snark_account_seq_no_valid() {
        let target = create_test_account_id();
        let base_update = create_test_snark_update();
        let tx_seq_no = base_update.operation().seq_no();
        let tx = create_test_snark_tx_from_update(
            target,
            base_update,
            create_test_constraints_with_slots(Some(50), Some(150)),
        );

        // Create state with snark account expecting the same seq_no as tx (next-expected semantics)
        let state_accessor = create_test_ol_state_with_snark_account(target, tx_seq_no, 100);

        // Empty account state - no pending transactions in mempool, so validates against chain
        // state
        let account_state = HashMap::new();

        let result = validate_transaction(tx.compute_txid(), &tx, &state_accessor, &account_state);
        assert!(result.is_ok());
    }

    #[test]
    fn test_snark_account_seq_no_invalid() {
        let target = create_test_account_id();
        let base_update = create_test_snark_update();
        let tx_seq_no = base_update.operation().seq_no();

        let tx = create_test_snark_tx_from_update(
            target,
            base_update.clone(),
            create_test_constraints_with_slots(Some(50), Some(150)),
        );

        // Test case 1: Transaction seq_no SMALLER than account seq_no (already used)
        // Account has seq_no = tx_seq_no + 1, transaction has seq_no = tx_seq_no
        // tx_seq_no < account_seq_no → UsedSequenceNumber
        let account_seq_no = tx_seq_no + 1;
        let state_accessor = create_test_ol_state_with_snark_account(target, account_seq_no, 100);

        let account_state = HashMap::new();

        let result = validate_transaction(tx.compute_txid(), &tx, &state_accessor, &account_state);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            OLMempoolError::UsedSequenceNumber { .. }
        ));

        // Test case 2: Transaction seq_no GREATER than mempool range (creates gap)
        // Account has seq_no = 5 on-chain
        // Mempool has pending transactions with seq_no 5, 6, 7 (so max_seq_no = 7)
        // New transaction has seq_no = 10 (gap: missing 8, 9)
        // tx_seq_no > max_seq_no + 1 → SequenceNumberGap
        let account_seq_no_onchain = 5;
        let tx2 = create_test_snark_tx_with_seq_no_and_slots(1, 10, Some(50), Some(150));
        let target2 = tx_target(&tx2);

        let state_accessor2 =
            create_test_ol_state_with_snark_account(target2, account_seq_no_onchain, 100);

        // Create account_state with pending transactions (seq_no 5, 6, 7)
        // We need actual transactions to get their txids
        let pending_tx5 = create_test_snark_tx_with_seq_no_and_slots(1, 5, Some(50), Some(150));
        let pending_tx6 = create_test_snark_tx_with_seq_no_and_slots(1, 6, Some(50), Some(150));
        let pending_tx7 = create_test_snark_tx_with_seq_no_and_slots(1, 7, Some(50), Some(150));

        let mut account_state2 = HashMap::new();
        account_state2.insert(
            target2,
            AccountMempoolState {
                txids: BTreeSet::from([
                    pending_tx5.compute_txid(),
                    pending_tx6.compute_txid(),
                    pending_tx7.compute_txid(),
                ]),
                seq_nos: BTreeSet::from([5, 6, 7]),
            },
        );

        let result2 =
            validate_transaction(tx2.compute_txid(), &tx2, &state_accessor2, &account_state2);
        assert!(result2.is_err());
        assert!(matches!(
            result2.unwrap_err(),
            OLMempoolError::SequenceNumberGap { .. }
        ));
    }
}
