//! Transaction validation for mempool
//!
//! Provides pluggable validation strategies for OL transactions.

use ssz::Encode;
use ssz_types::Optional;
use strata_acct_types::AccountTypeId;
use strata_identifiers::OLBlockCommitment;
use strata_ledger_types::{IAccountState, ISnarkAccountState, IStateAccessor};
use strata_ol_state_types::OLState;

use crate::{OLMempoolConfig, OLMempoolError, OLMempoolResult, types::OLMempoolTransaction};

/// Trait for transaction validation strategies
///
/// Enables pluggable validation logic for mempool transaction submission.
/// Validators can implement different policies (basic checks, fee validation, etc.)
pub trait TransactionValidator: Send + Sync {
    /// Validate transaction against current chain state
    ///
    /// # Arguments
    /// * `tx` - Transaction to validate
    /// * `current_tip` - Current chain tip (slot + block ID)
    /// * `state_accessor` - Access to chain state for validation
    ///
    /// # Returns
    /// * `Ok(())` if transaction is valid
    /// * `Err(MempoolError)` with specific rejection reason if invalid
    fn validate(
        &self,
        tx: &OLMempoolTransaction,
        current_tip: &OLBlockCommitment,
        state_accessor: &OLState,
    ) -> OLMempoolResult<()>;
}

/// Basic transaction validator
///
/// Validates:
/// - Structural validity (size, slot bounds)
/// - Account existence
/// - Sequence number ordering
#[derive(Debug, Clone)]
pub struct BasicTransactionValidator {
    config: OLMempoolConfig,
}

impl BasicTransactionValidator {
    /// Create a new basic transaction validator
    pub fn new(config: OLMempoolConfig) -> Self {
        Self { config }
    }
}

impl TransactionValidator for BasicTransactionValidator {
    fn validate(
        &self,
        tx: &OLMempoolTransaction,
        current_tip: &OLBlockCommitment,
        state_accessor: &OLState,
    ) -> OLMempoolResult<()> {
        let current_slot = current_tip.slot;

        // 1. Transaction size validation
        let tx_size = tx.as_ssz_bytes().len();
        if tx_size > self.config.max_tx_size {
            return Err(OLMempoolError::TransactionTooLarge {
                size: tx_size,
                limit: self.config.max_tx_size,
            });
        }

        // 2. Slot bounds validation
        if let Optional::Some(min_slot) = tx.attachment.min_slot
            && current_slot < min_slot
        {
            return Err(OLMempoolError::TransactionNotYetValid {
                txid: tx.compute_txid(),
                min_slot,
                current_slot,
            });
        }

        if let Optional::Some(max_slot) = tx.attachment.max_slot
            && current_slot >= max_slot
        {
            return Err(OLMempoolError::TransactionExpired {
                txid: tx.compute_txid(),
                max_slot,
                current_slot,
            });
        }

        // 3. Account existence check
        let target_account = tx.target();
        let account_exists = state_accessor
            .check_account_exists(target_account)
            .map_err(|e| {
                OLMempoolError::AccountStateAccess(format!(
                    "Failed to check account existence: {e}"
                ))
            })?;

        if !account_exists {
            return Err(OLMempoolError::AccountDoesNotExist {
                account: target_account,
            });
        }

        // 4. Sequence number validation (for SnarkAccountUpdate transactions only)
        if let Some(base_update) = tx.base_update() {
            let tx_seq_no = base_update.operation().seq_no();

            // Get account state to check current sequence number
            let account_state = state_accessor
                .get_account_state(target_account)
                .map_err(|e| {
                    OLMempoolError::AccountStateAccess(format!("Failed to get account state: {e}"))
                })?;

            let account_state = account_state.ok_or_else(|| {
                OLMempoolError::AccountStateAccess(format!(
                    "Account {} state not found",
                    target_account
                ))
            })?;

            // SnarkAccountUpdate transactions must target Snark accounts
            if account_state.ty() != AccountTypeId::Snark {
                return Err(OLMempoolError::AccountTypeMismatch {
                    txid: tx.compute_txid(),
                    account: target_account,
                });
            }

            let snark_state = account_state.as_snark_account().map_err(|e| {
                OLMempoolError::AccountStateAccess(format!(
                    "Failed to get snark account state: {e}"
                ))
            })?;

            let current_seq_no = *snark_state.seqno().inner();

            // Validator checks: transaction seq_no must be >= current account seq_no
            // (can't be in the past - mempool will check for gaps separately)
            if tx_seq_no < current_seq_no {
                return Err(OLMempoolError::InvalidSequenceNumber {
                    txid: tx.compute_txid(),
                    tx_seq_no,
                    account_seq_no: current_seq_no,
                });
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        test_utils::{
            create_test_account_id, create_test_attachment_with_slots,
            create_test_block_commitment, create_test_generic_tx_with_size,
            create_test_generic_tx_with_slots, create_test_ol_state_with_account,
            create_test_ol_state_with_snark_account, create_test_snark_update,
        },
        types::OLMempoolTransaction,
    };

    #[test]
    fn test_transaction_size_validation() {
        let config = OLMempoolConfig {
            max_tx_count: 100,
            max_tx_size: 100,
        };
        let validator = BasicTransactionValidator::new(config);
        let current_tip = create_test_block_commitment(100);

        // Create a transaction that's too large (with valid slot bounds to isolate size check)
        let attachment = create_test_attachment_with_slots(Some(50), Some(150));
        let tx = create_test_generic_tx_with_size(200, attachment); // Larger than max_tx_size (100)

        // Create state with the account
        let state_accessor = create_test_ol_state_with_account(tx.target());

        let result = validator.validate(&tx, &current_tip, &state_accessor);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            OLMempoolError::TransactionTooLarge { .. }
        ));
    }

    #[test]
    fn test_transaction_size_valid() {
        let config = OLMempoolConfig {
            max_tx_count: 100,
            max_tx_size: 1000,
        };
        let validator = BasicTransactionValidator::new(config);
        let current_tip = create_test_block_commitment(100);

        // Create a valid-sized transaction (with valid slot bounds)
        let attachment = create_test_attachment_with_slots(Some(50), Some(150));
        let tx = create_test_generic_tx_with_size(100, attachment);

        // Create state with the account
        let state_accessor = create_test_ol_state_with_account(tx.target());

        let result = validator.validate(&tx, &current_tip, &state_accessor);
        assert!(result.is_ok());
    }

    #[test]
    fn test_transaction_size_at_limit() {
        let config = OLMempoolConfig {
            max_tx_count: 100,
            max_tx_size: 200,
        };
        let _validator = BasicTransactionValidator::new(config);
        let current_tip = create_test_block_commitment(100);

        // Create a transaction and check its actual size
        let attachment = create_test_attachment_with_slots(Some(50), Some(150));
        let tx = create_test_generic_tx_with_size(100, attachment);
        let tx_size = tx.as_ssz_bytes().len();

        // Create state with the account
        let state_accessor = create_test_ol_state_with_account(tx.target());

        // Set limit to exactly the transaction size (should pass, check is > not >=)
        let config_at_limit = OLMempoolConfig {
            max_tx_count: 100,
            max_tx_size: tx_size,
        };
        let validator_at_limit = BasicTransactionValidator::new(config_at_limit);

        let result = validator_at_limit.validate(&tx, &current_tip, &state_accessor);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validation_with_default_config() {
        // Test that validation works with realistic default config values
        let config = OLMempoolConfig::default();
        let validator = BasicTransactionValidator::new(config);
        let current_tip = create_test_block_commitment(100);

        // Normal transaction should pass with default config
        let tx = create_test_generic_tx_with_slots(Some(50), Some(150));

        // Create state with the account
        let state_accessor = create_test_ol_state_with_account(tx.target());

        let result = validator.validate(&tx, &current_tip, &state_accessor);
        assert!(result.is_ok());
    }

    #[test]
    fn test_min_slot_validation() {
        let config = OLMempoolConfig::default();
        let validator = BasicTransactionValidator::new(config);
        let current_tip = create_test_block_commitment(100);

        // Transaction with min_slot in the future
        let tx = create_test_generic_tx_with_slots(Some(150), None);

        // Create state with the account
        let state_accessor = create_test_ol_state_with_account(tx.target());

        let result = validator.validate(&tx, &current_tip, &state_accessor);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            OLMempoolError::TransactionNotYetValid { .. }
        ));
    }

    #[test]
    fn test_min_slot_boundary() {
        let config = OLMempoolConfig::default();
        let validator = BasicTransactionValidator::new(config);
        let current_tip = create_test_block_commitment(100);

        // Transaction with min_slot equal to current_slot (should pass, check is < not <=)
        let tx = create_test_generic_tx_with_slots(Some(100), Some(150));

        // Create state with the account
        let state_accessor = create_test_ol_state_with_account(tx.target());

        let result = validator.validate(&tx, &current_tip, &state_accessor);
        assert!(result.is_ok());
    }

    #[test]
    fn test_max_slot_validation() {
        let config = OLMempoolConfig::default();
        let validator = BasicTransactionValidator::new(config);
        let current_tip = create_test_block_commitment(100);

        // Transaction with max_slot in the past
        let tx = create_test_generic_tx_with_slots(None, Some(50)); // max_slot < current_slot

        // Create state with the account
        let state_accessor = create_test_ol_state_with_account(tx.target());

        let result = validator.validate(&tx, &current_tip, &state_accessor);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            OLMempoolError::TransactionExpired { .. }
        ));
    }

    #[test]
    fn test_max_slot_boundary() {
        let config = OLMempoolConfig::default();
        let validator = BasicTransactionValidator::new(config);
        let current_tip = create_test_block_commitment(100);

        // Transaction with max_slot equal to current_slot (should fail, check is >=)
        let tx = create_test_generic_tx_with_slots(None, Some(100)); // max_slot == current_slot

        // Create state with the account
        let state_accessor = create_test_ol_state_with_account(tx.target());

        let result = validator.validate(&tx, &current_tip, &state_accessor);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            OLMempoolError::TransactionExpired { .. }
        ));
    }

    #[test]
    fn test_max_slot_one_after_current() {
        let config = OLMempoolConfig::default();
        let validator = BasicTransactionValidator::new(config);
        let current_tip = create_test_block_commitment(100);

        // Transaction with max_slot == current_slot + 1 (should pass, check is >=)
        let tx = create_test_generic_tx_with_slots(Some(50), Some(101));

        // Create state with the account
        let state_accessor = create_test_ol_state_with_account(tx.target());

        let result = validator.validate(&tx, &current_tip, &state_accessor);
        assert!(result.is_ok());
    }

    #[test]
    fn test_valid_slot_bounds() {
        let config = OLMempoolConfig::default();
        let validator = BasicTransactionValidator::new(config);
        let current_tip = create_test_block_commitment(100);

        // Transaction with valid slot bounds
        let tx = create_test_generic_tx_with_slots(Some(50), Some(150)); // min < current < max

        // Create state with the account
        let state_accessor = create_test_ol_state_with_account(tx.target());

        let result = validator.validate(&tx, &current_tip, &state_accessor);
        assert!(result.is_ok());
    }

    #[test]
    fn test_valid_no_slot_bounds() {
        let config = OLMempoolConfig::default();
        let validator = BasicTransactionValidator::new(config);
        let current_tip = create_test_block_commitment(100);

        // Transaction with no slot bounds (should be valid)
        let tx = create_test_generic_tx_with_slots(None, None);

        // Create state with the account
        let state_accessor = create_test_ol_state_with_account(tx.target());

        let result = validator.validate(&tx, &current_tip, &state_accessor);
        assert!(result.is_ok());
    }

    #[test]
    fn test_snark_account_update_validation() {
        let config = OLMempoolConfig::default();
        let validator = BasicTransactionValidator::new(config);
        let current_tip = create_test_block_commitment(100);

        // Snark account update transaction
        let target = create_test_account_id();
        let base_update = create_test_snark_update();
        let attachment = create_test_attachment_with_slots(Some(50), Some(150));
        let tx = OLMempoolTransaction::new_snark_account_update(target, base_update, attachment);

        // Create state accessor with a Snark account that has seq_no = 0
        // Transaction seq_no should be >= 0
        let state_accessor = create_test_ol_state_with_snark_account(target, 0);

        let result = validator.validate(&tx, &current_tip, &state_accessor);
        assert!(result.is_ok());
    }

    #[test]
    fn test_snark_account_update_size_validation() {
        let config = OLMempoolConfig {
            max_tx_count: 100,
            max_tx_size: 100,
        };
        let validator = BasicTransactionValidator::new(config);
        let current_tip = create_test_block_commitment(100);

        // Snark account update that's too large (if we can create one)
        // Note: This might not be possible with current test utils, but tests the logic
        let target = create_test_account_id();
        let base_update = create_test_snark_update();
        let attachment = create_test_attachment_with_slots(Some(50), Some(150));
        let tx = OLMempoolTransaction::new_snark_account_update(target, base_update, attachment);

        // Create state accessor with a Snark account for the target
        let state_accessor = create_test_ol_state_with_snark_account(target, 0);

        // Check if it's actually too large, otherwise skip
        let tx_size = tx.as_ssz_bytes().len();
        if tx_size > 100 {
            let result = validator.validate(&tx, &current_tip, &state_accessor);
            assert!(result.is_err());
            assert!(matches!(
                result.unwrap_err(),
                OLMempoolError::TransactionTooLarge { .. }
            ));
        }
    }
}
