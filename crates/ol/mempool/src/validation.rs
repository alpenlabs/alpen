//! Transaction validation for mempool.
//!
//! Provides pluggable validation strategies to check transaction validity before inclusion.

use strata_ol_chain_types_new::OLTransaction;

use crate::{MempoolError, MempoolResult, MempoolTxMetadata};

/// Strategy for validating transactions before mempool inclusion.
pub trait TransactionValidator: Send + Sync {
    /// Validate a transaction for mempool inclusion.
    ///
    /// Returns Ok(()) if the transaction is valid, or an error describing the issue.
    fn validate(
        &self,
        tx: &OLTransaction,
        metadata: &MempoolTxMetadata,
        current_slot: u64,
    ) -> MempoolResult<()>;

    /// Name of the validator (for logging/metrics).
    fn name(&self) -> &'static str;
}

/// Basic validator that checks fundamental transaction properties.
///
/// Validates:
/// - Transaction size within limits
/// - min_slot <= max_slot (if both present)
/// - Transaction not expired (max_slot >= current_slot)
/// - Transaction not too early (min_slot > current_slot)
#[derive(Debug, Clone, Copy)]
pub struct BasicValidator {
    /// Maximum transaction size in bytes.
    pub max_tx_size: usize,
}

impl BasicValidator {
    /// Create a new basic validator with the given size limit.
    pub fn new(max_tx_size: usize) -> Self {
        Self { max_tx_size }
    }
}

impl TransactionValidator for BasicValidator {
    fn validate(
        &self,
        tx: &OLTransaction,
        metadata: &MempoolTxMetadata,
        current_slot: u64,
    ) -> MempoolResult<()> {
        // Check transaction size
        if metadata.size_bytes > self.max_tx_size {
            return Err(MempoolError::TransactionTooLarge {
                size: metadata.size_bytes,
                max: self.max_tx_size,
            });
        }

        // Validate slot ranges
        let min_slot = tx.extra().min_slot();
        let max_slot = tx.extra().max_slot();

        // If both min_slot and max_slot are present, min must be <= max
        if let (Some(min), Some(max)) = (min_slot, max_slot)
            && min > max
        {
            return Err(MempoolError::InvalidTransaction(format!(
                "Invalid slot range: min_slot ({min}) > max_slot ({max})",
            )));
        }

        // Check if transaction is too early
        if let Some(min) = min_slot
            && min > current_slot
        {
            return Err(MempoolError::TooEarly {
                min_slot: min,
                current_slot,
            });
        }

        // Check if transaction is expired
        if let Some(max) = max_slot
            && max < current_slot
        {
            return Err(MempoolError::Expired { max_slot: max });
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        "basic"
    }
}

#[cfg(test)]
mod tests {
    use strata_acct_types::AccountId;
    use strata_ol_chain_types_new::{TransactionExtra, TransactionPayload};

    use super::*;

    fn create_test_tx(min_slot: Option<u64>, max_slot: Option<u64>) -> OLTransaction {
        OLTransaction::new(
            TransactionPayload::GenericAccountMessage {
                target: AccountId::new([1u8; 32]),
                payload: vec![1, 2, 3],
            },
            TransactionExtra::new(min_slot, max_slot),
        )
    }

    fn create_test_metadata(size: usize) -> MempoolTxMetadata {
        MempoolTxMetadata {
            entry_slot: 100,
            entry_time: 0,
            size_bytes: size,
        }
    }

    #[test]
    fn test_valid_transaction() {
        let validator = BasicValidator::new(1024 * 1024);
        let tx = create_test_tx(Some(50), Some(150));
        let metadata = create_test_metadata(1000);

        // Current slot is 100, within [50, 150] range
        let result = validator.validate(&tx, &metadata, 100);
        assert!(result.is_ok());
    }

    #[test]
    fn test_transaction_too_large() {
        let validator = BasicValidator::new(100);
        let tx = create_test_tx(None, None);
        let metadata = create_test_metadata(200); // Exceeds limit

        let result = validator.validate(&tx, &metadata, 100);
        assert!(matches!(
            result,
            Err(MempoolError::TransactionTooLarge {
                size: 200,
                max: 100
            })
        ));
    }

    #[test]
    fn test_transaction_too_early() {
        let validator = BasicValidator::new(1024 * 1024);
        let tx = create_test_tx(Some(200), None);
        let metadata = create_test_metadata(100);

        // Current slot is 100, but min_slot is 200
        let result = validator.validate(&tx, &metadata, 100);
        assert!(matches!(
            result,
            Err(MempoolError::TooEarly {
                min_slot: 200,
                current_slot: 100
            })
        ));
    }

    #[test]
    fn test_transaction_expired() {
        let validator = BasicValidator::new(1024 * 1024);
        let tx = create_test_tx(None, Some(50));
        let metadata = create_test_metadata(100);

        // Current slot is 100, but max_slot is 50
        let result = validator.validate(&tx, &metadata, 100);
        assert!(matches!(
            result,
            Err(MempoolError::Expired { max_slot: 50 })
        ));
    }

    #[test]
    fn test_invalid_slot_range() {
        let validator = BasicValidator::new(1024 * 1024);
        let tx = create_test_tx(Some(150), Some(100)); // min > max
        let metadata = create_test_metadata(100);

        let result = validator.validate(&tx, &metadata, 100);
        assert!(matches!(result, Err(MempoolError::InvalidTransaction(_))));
    }

    #[test]
    fn test_transaction_without_slots() {
        let validator = BasicValidator::new(1024 * 1024);
        let tx = create_test_tx(None, None);
        let metadata = create_test_metadata(100);

        // No slot restrictions - should be valid at any slot
        let result = validator.validate(&tx, &metadata, 100);
        assert!(result.is_ok());
    }
}
