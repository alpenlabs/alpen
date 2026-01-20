//! Utility functions for EVM block execution.
//!
//! This module contains utility functions used during block execution that don't
//! belong to any specific type.

use alloy_consensus::Block as AlloyBlock;
use alpen_reth_evm::subject_to_address;
use reth_evm::execute::BlockExecutionOutput;
use reth_primitives::{Receipt as EthereumReceipt, RecoveredBlock, TransactionSigned};
use reth_primitives_traits::Block;
use reth_trie::{HashedPostState, KeccakKeyHasher};
use strata_ee_acct_types::{EnvError, EnvResult, ExecPayload};
use strata_ee_chain_types::BlockInputs;

use crate::types::EvmBlock;

/// Builds an Alloy block from exec payload and recovers transaction senders.
///
/// This constructs an AlloyBlock from the header and body in the exec payload,
/// then recovers the sender addresses from transaction signatures.
pub(crate) fn build_and_recover_block(
    exec_payload: &ExecPayload<'_, EvmBlock>,
) -> EnvResult<RecoveredBlock<AlloyBlock<TransactionSigned>>> {
    let header = exec_payload.header_intrinsics().clone();
    let body = exec_payload.body().body().clone();

    // Build block using alloy_consensus types
    let alloy_block = AlloyBlock { header, body };

    // Recover transaction senders from signatures
    alloy_block
        .try_into_recovered()
        .map_err(|_| EnvError::InvalidBlock)
}

/// Converts execution output to HashedPostState for state updates.
pub(crate) fn compute_hashed_post_state(
    execution_output: BlockExecutionOutput<EthereumReceipt>,
    _block_number: u64,
) -> HashedPostState {
    HashedPostState::from_bundle_state::<KeccakKeyHasher>(&execution_output.state.state)
}

/// Converts satoshis to gwei for EVM compatibility.
///
/// In Alpen: 1 BTC = 10^8 sats = 10^9 gwei
/// Therefore: 1 sat = 10 gwei
///
/// Per EIP-4895, withdrawal amounts are stored in Gwei (not Wei).
fn sats_to_gwei(sats: u64) -> Option<u64> {
    sats.checked_mul(10)
}

/// Validates that deposits from BlockInputs match the withdrawals field in the block.
///
/// In Alpen, the EIP-4895 withdrawals field is hijacked to represent deposits from the
/// orchestration layer. This function ensures that the authenticated deposits in BlockInputs
/// match what's committed in the block's withdrawals field.
///
/// # Warning
/// **Deposits and withdrawals must be in the same order.** This function performs a
/// pairwise comparison using `zip()`, so the nth deposit must match the nth withdrawal.
/// Any reordering will cause validation to fail.
///
/// # Mapping
/// - `Withdrawal.address` ← last 20 bytes of `SubjectDepositData.dest` (SubjectId)
/// - `Withdrawal.amount` ← `SubjectDepositData.value` in Gwei
/// - `Withdrawal.index` and `Withdrawal.validator_index` are ignored (not meaningful for deposits)
///
/// # Returns
/// - `Ok(())` if deposits match
/// - `Err(EnvError::InvalidBlock)` if there's a mismatch in count, address, or amount
pub(crate) fn validate_deposits_against_block(
    block: &RecoveredBlock<AlloyBlock<TransactionSigned>>,
    inputs: &BlockInputs,
) -> EnvResult<()> {
    // Get withdrawals from the block body (this is where deposits are stored)
    // Access the sealed block's body withdrawals through the nested structure
    let block_withdrawals = block
        .sealed_block()
        .body()
        .withdrawals
        .as_ref()
        .map(|w| w.as_slice())
        .unwrap_or(&[]);

    let subject_deposits = inputs.subject_deposits();

    // Check counts match
    if block_withdrawals.len() != subject_deposits.len() {
        return Err(EnvError::InvalidBlock);
    }

    // Validate each deposit matches the corresponding withdrawal
    for (withdrawal, deposit) in block_withdrawals.iter().zip(subject_deposits.iter()) {
        // Convert SubjectId to Address - returns None if first 12 bytes aren't zero
        let expected_address = subject_to_address(&deposit.dest())
            .ok_or_else(|| EnvError::InvalidDepositAddress(deposit.dest()))?;

        // Convert satoshis to gwei (1 sat = 10 gwei, per 1 BTC = 10^8 sats = 10^9 gwei)
        let expected_amount =
            sats_to_gwei(deposit.value().to_sat()).ok_or(EnvError::InvalidBlock)?;

        // Validate address and amount match
        if withdrawal.address != expected_address || withdrawal.amount != expected_amount {
            return Err(EnvError::InvalidBlock);
        }

        // Note: withdrawal.index and withdrawal.validator_index are not validated
        // as they are not meaningful in the deposit context
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use alloy_consensus::{BlockBody, Header};
    use alloy_eips::eip4895::Withdrawal;
    use reth_primitives::RecoveredBlock;
    use revm_primitives::Address;
    use strata_acct_types::{BitcoinAmount, SubjectId};
    use strata_ee_chain_types::{BlockInputs, SubjectDepositData};

    use super::*;

    #[test]
    fn test_validate_deposits_valid_match() {
        // Create valid subject ID: [0x00..0x00 (12 bytes), address (20 bytes)]
        let mut subject_bytes = [0u8; 32];
        subject_bytes[31] = 0x42; // Last byte of address
        let subject_id = SubjectId::new(subject_bytes);
        let expected_address = subject_to_address(&subject_id).expect("valid subject ID");

        // Create a block with matching withdrawal
        let header = Header::default();
        let body = BlockBody {
            transactions: vec![],
            ommers: vec![],
            withdrawals: Some(
                vec![Withdrawal {
                    index: 0,
                    validator_index: 0,
                    address: expected_address,
                    amount: 100, // 10 sats * 10 = 100 gwei
                }]
                .into(),
            ),
        };
        let block = AlloyBlock { header, body };
        let recovered_block: RecoveredBlock<AlloyBlock<TransactionSigned>> =
            block.try_into_recovered().unwrap();

        // Create matching deposit input
        let mut inputs = BlockInputs::new_empty();
        let deposit = SubjectDepositData::new(subject_id, BitcoinAmount::from_sat(10));
        inputs.add_subject_deposit(deposit);

        // Should succeed - perfect match
        let result = validate_deposits_against_block(&recovered_block, &inputs);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_deposits_address_mismatch() {
        // Valid subject ID but different address
        let mut subject_bytes = [0u8; 32];
        subject_bytes[31] = 0xff; // Different address
        let subject_id = SubjectId::new(subject_bytes);

        // Create a block with different address
        let header = Header::default();
        let body = BlockBody {
            transactions: vec![],
            ommers: vec![],
            withdrawals: Some(
                vec![Withdrawal {
                    index: 0,
                    validator_index: 0,
                    address: Address::ZERO, // Wrong address
                    amount: 100,
                }]
                .into(),
            ),
        };
        let block = AlloyBlock { header, body };
        let recovered_block: RecoveredBlock<AlloyBlock<TransactionSigned>> =
            block.try_into_recovered().unwrap();

        // Create deposit with different address
        let mut inputs = BlockInputs::new_empty();
        let deposit = SubjectDepositData::new(subject_id, BitcoinAmount::from_sat(10));
        inputs.add_subject_deposit(deposit);

        // Should fail - address mismatch
        let result = validate_deposits_against_block(&recovered_block, &inputs);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_deposits_invalid_subject_id() {
        // Invalid subject ID with non-zero padding in first 12 bytes
        let subject_id = SubjectId::new([0xff; 32]);

        // Create a block
        let header = Header::default();
        let body = BlockBody {
            transactions: vec![],
            ommers: vec![],
            withdrawals: Some(
                vec![Withdrawal {
                    index: 0,
                    validator_index: 0,
                    address: Address::from([0xff; 20]),
                    amount: 100,
                }]
                .into(),
            ),
        };
        let block = AlloyBlock { header, body };
        let recovered_block: RecoveredBlock<AlloyBlock<TransactionSigned>> =
            block.try_into_recovered().unwrap();

        // Create deposit with invalid subject ID
        let mut inputs = BlockInputs::new_empty();
        let deposit = SubjectDepositData::new(subject_id, BitcoinAmount::from_sat(10));
        inputs.add_subject_deposit(deposit);

        // Should fail - invalid subject ID
        let result = validate_deposits_against_block(&recovered_block, &inputs);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_deposits_amount_mismatch() {
        let mut subject_bytes = [0u8; 32];
        subject_bytes[31] = 0x42;
        let subject_id = SubjectId::new(subject_bytes);
        let expected_address = subject_to_address(&subject_id).expect("valid subject ID");

        // Create a block with wrong amount
        let header = Header::default();
        let body = BlockBody {
            transactions: vec![],
            ommers: vec![],
            withdrawals: Some(
                vec![Withdrawal {
                    index: 0,
                    validator_index: 0,
                    address: expected_address,
                    amount: 200, // Wrong amount (should be 100)
                }]
                .into(),
            ),
        };
        let block = AlloyBlock { header, body };
        let recovered_block: RecoveredBlock<AlloyBlock<TransactionSigned>> =
            block.try_into_recovered().unwrap();

        // Create deposit with different amount
        let mut inputs = BlockInputs::new_empty();
        let deposit = SubjectDepositData::new(subject_id, BitcoinAmount::from_sat(10)); // 10 sats = 100 gwei
        inputs.add_subject_deposit(deposit);

        // Should fail - amount mismatch
        let result = validate_deposits_against_block(&recovered_block, &inputs);
        assert!(result.is_err());
    }
}
