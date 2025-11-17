use bitcoin::{ScriptBuf, Transaction, XOnlyPublicKey};
use strata_primitives::l1::BitcoinXOnlyPublicKey;

use crate::errors::CommitInputError;

/// The index of the Claim output that must be spent by the Commit transaction.
/// This is the first output (index 0) of the Claim transaction.
pub const CLAIM_OUTPUT_INDEX: usize = 0;

/// The index of the N/N continuation output in the Commit transaction.
/// This output must be locked to the N/N aggregated operator key.
pub const COMMIT_NN_OUTPUT_INDEX: usize = 1;

// TODO:PG: add validatation that this transaction was created by spending N/N UTXO.

/// Validates that the second output of the commit transaction is locked to the N/N
/// aggregated operator key.
///
/// This function verifies that the commit transaction's second output (at index 1) is a P2TR
/// output locked to the provided aggregated operator public key with no merkle root
/// (key-spend only). This ensures the N/N chain continues for subsequent transactions.
///
/// # Parameters
///
/// - `tx` - The commit transaction to validate
/// - `operators_agg_pubkey` - The aggregated operator public key that should control the output
///
/// # Returns
///
/// - `Ok(())` - If the second output is properly locked to the operator key
/// - `Err(CommitInputError)` - If the output is missing, has wrong script type, or wrong key
pub fn validate_commit_nn_output(
    tx: &Transaction,
    operators_agg_pubkey: &BitcoinXOnlyPublicKey,
) -> Result<(), CommitInputError> {
    // Get the second output at the expected index
    let nn_output = tx
        .output
        .get(COMMIT_NN_OUTPUT_INDEX)
        .ok_or(CommitInputError::MissingSecondOutput)?;

    // Extract the internal key from the P2TR script
    let secp = secp256k1::SECP256K1;
    let operators_pubkey = XOnlyPublicKey::from_slice(operators_agg_pubkey.inner().as_bytes())
        .map_err(|_| CommitInputError::InvalidOperatorKey)?;

    // Create expected P2TR script with no merkle root (key-spend only)
    let expected_script = ScriptBuf::new_p2tr(secp, operators_pubkey, None);

    // Verify the second output script matches the expected P2TR script
    if nn_output.script_pubkey != expected_script {
        return Err(CommitInputError::WrongSecondOutputLock);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use bitcoin::{
        Amount, ScriptBuf, Transaction, TxOut,
        absolute::LockTime,
        secp256k1::{Secp256k1, SecretKey},
        transaction::Version,
    };
    use rand::Rng;
    use strata_crypto::{EvenSecretKey, test_utils::schnorr::create_agg_pubkey_from_privkeys};
    use strata_primitives::{buf::Buf32, l1::BitcoinXOnlyPublicKey};

    use super::*;

    // Helper function to create test operator keys with proper MuSig2 aggregation
    fn create_test_operators() -> (BitcoinXOnlyPublicKey, Vec<EvenSecretKey>) {
        let mut rng = secp256k1::rand::thread_rng();
        let num_operators = rng.gen_range(2..=5);

        // Generate random operator keys
        let operators_privkeys: Vec<EvenSecretKey> = (0..num_operators)
            .map(|_| SecretKey::new(&mut rng).into())
            .collect();

        // Use helper function to aggregate keys
        let aggregated_xonly = create_agg_pubkey_from_privkeys(&operators_privkeys);
        let operators_pubkey = BitcoinXOnlyPublicKey::new(Buf32::new(aggregated_xonly.serialize()))
            .expect("Valid aggregated public key");

        (operators_pubkey, operators_privkeys)
    }

    #[test]
    fn test_validate_commit_nn_output_success() {
        let (operators_pubkey, _) = create_test_operators();
        let secp = Secp256k1::new();

        let operators_xonly = XOnlyPublicKey::from_slice(operators_pubkey.inner().as_bytes())
            .expect("Valid xonly pubkey");
        let nn_script = ScriptBuf::new_p2tr(&secp, operators_xonly, None);

        // Create a commit transaction with proper N/N output at index 1
        let tx = Transaction {
            version: Version(2),
            lock_time: LockTime::ZERO,
            input: vec![],
            output: vec![
                // Index 0: OP_RETURN (not validated by this function)
                TxOut {
                    value: Amount::from_sat(0),
                    script_pubkey: ScriptBuf::new(),
                },
                // Index 1: N/N output
                TxOut {
                    value: Amount::from_sat(10000),
                    script_pubkey: nn_script,
                },
            ],
        };

        let result = validate_commit_nn_output(&tx, &operators_pubkey);
        assert!(result.is_ok(), "Valid N/N output should pass validation");
    }

    #[test]
    fn test_validate_commit_nn_output_missing() {
        let (operators_pubkey, _) = create_test_operators();

        // Create a commit transaction with only one output
        let tx = Transaction {
            version: Version(2),
            lock_time: LockTime::ZERO,
            input: vec![],
            output: vec![TxOut {
                value: Amount::from_sat(0),
                script_pubkey: ScriptBuf::new(),
            }],
        };

        let err = validate_commit_nn_output(&tx, &operators_pubkey).unwrap_err();
        assert!(matches!(err, CommitInputError::MissingSecondOutput));
    }

    #[test]
    fn test_validate_commit_nn_output_wrong_script() {
        let (operators_pubkey, _) = create_test_operators();

        // Create a commit transaction with wrong script at index 1
        let tx = Transaction {
            version: Version(2),
            lock_time: LockTime::ZERO,
            input: vec![],
            output: vec![
                // Index 0: OP_RETURN
                TxOut {
                    value: Amount::from_sat(0),
                    script_pubkey: ScriptBuf::new(),
                },
                // Index 1: Wrong script (empty instead of P2TR)
                TxOut {
                    value: Amount::from_sat(10000),
                    script_pubkey: ScriptBuf::new(),
                },
            ],
        };

        let err = validate_commit_nn_output(&tx, &operators_pubkey).unwrap_err();
        assert!(matches!(err, CommitInputError::WrongSecondOutputLock));
    }
}
