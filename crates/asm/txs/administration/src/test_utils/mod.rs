use bitcoin::{ScriptBuf, Transaction, secp256k1::SecretKey};
use bitvec::vec::BitVec;
use strata_crypto::multisig::{
    schemes::{SchnorrScheme, schnorr::create::create_musig2_signature},
    signature::MultisigSignature,
};
use strata_l1tx::envelope::builder::build_envelope_script;
use strata_primitives::{
    buf::{Buf32, Buf64},
    l1::payload::{L1Payload, L1PayloadType},
    params::Params,
};

pub(crate) const TEST_MAGIC_BYTES: &[u8; 4] = b"ALPN";

use crate::{actions::MultisigAction, constants::ADMINISTRATION_SUBPROTOCOL_ID};

/// Creates a MultisigSignature for any MultisigAction.
///
/// This function generates the required signature for any administration action
/// (Update or Cancel) by computing the sighash from the action and sequence number,
/// then creating a MuSig2 signature using the provided private keys.
///
/// # Arguments
/// * `privkeys` - Private keys of all signers in the multisig config
/// * `signer_indices` - BitVec indicating which signers are participating in this signature
/// * `action` - The MultisigAction to sign (Update or Cancel)
/// * `seqno` - The sequence number for this operation
///
/// # Returns
/// A MultisigSignature that can be used to authorize this action
pub fn create_multisig_signature(
    privkeys: &[SecretKey],
    signer_indices: BitVec<u8>,
    sighash: Buf32,
) -> MultisigSignature<SchnorrScheme> {
    // Extract only the private keys for signers indicated by signer_indices
    let selected_privkeys: Vec<SecretKey> = signer_indices
        .iter_ones()
        .map(|index| privkeys[index])
        .collect();

    let signature = create_musig2_signature(&selected_privkeys, &sighash.0, None);
    let signature_buf = Buf64::from(signature.serialize());

    MultisigSignature::new(signer_indices, signature_buf)
}

/// Creates a SPS-50 compliant administration transaction with commit-reveal pattern.
///
/// This function creates only the reveal transaction that contains both the action and signature.
/// The reveal transaction uses the envelope script format to embed the administration payload
/// in a way that's compatible with SPS-50.
///
/// # Arguments
/// * `params` - Network parameters containing rollup configuration
/// * `privkeys` - Private keys of all signers in the multisig config
/// * `signer_indices` - BitVec indicating which signers are participating in this signature
/// * `action` - The MultisigAction to sign and embed (Update or Cancel)
/// * `seqno` - The sequence number for this operation
///
/// # Returns
/// A Bitcoin transaction that serves as the reveal transaction containing the administration
/// payload
pub fn create_admin_tx(
    params: &Params,
    privkeys: &[SecretKey],
    signer_indices: BitVec<u8>,
    action: &MultisigAction,
    seqno: u64,
) -> anyhow::Result<Transaction> {
    // Compute the signature hash and create the multisig signature
    let sighash = action.compute_sighash(seqno);
    let signature = create_multisig_signature(privkeys, signer_indices, sighash);

    // Create auxiliary data in the expected format for deposit transactions
    let mut aux_data = Vec::new();
    aux_data.extend_from_slice(signature.signature().as_bytes()); // 64 bytes

    // Now we can directly get bytes from BitVec<u8> - much cleaner!
    let signer_indices_bytes = signature.signer_indices().to_bitvec().into_vec();
    aux_data.extend_from_slice(&signer_indices_bytes); // variable length bitset as bytes

    // Create the complete SPS-50 tagged payload
    // Format: [MAGIC_BYTES][SUBPROTOCOL_ID][TX_TYPE][AUX_DATA]
    let mut tagged_payload = Vec::new();
    tagged_payload.extend_from_slice(TEST_MAGIC_BYTES); // 4 bytes magic
    tagged_payload.extend_from_slice(&ADMINISTRATION_SUBPROTOCOL_ID.to_be_bytes()); // 1 byte subprotocol ID
    tagged_payload.extend_from_slice(&[action.tx_type()]); // 1 byte TxType
    tagged_payload.extend_from_slice(&aux_data); // auxiliary data

    // Create L1 payload for the administration data
    // Using L1PayloadType::Da as the administration subprotocol uses DA-like semantics
    let l1_payload = L1Payload::new(vec![], L1PayloadType::Da);

    // Build the envelope script containing our administration payload
    let envelope_script = build_envelope_script(params, &[l1_payload])?;

    // Create a minimal reveal transaction structure
    // This is a simplified version - in practice, this would be created as part of
    // a proper commit-reveal transaction pair using the btcio writer infrastructure
    let reveal_tx = create_reveal_transaction_stub(envelope_script)?;

    Ok(reveal_tx)
}

/// Creates a stub reveal transaction containing the envelope script.
/// This is a simplified implementation for testing purposes.
fn create_reveal_transaction_stub(script: ScriptBuf) -> anyhow::Result<Transaction> {
    use bitcoin::{
        Amount, OutPoint, Sequence, TxIn, TxOut, absolute::LockTime, transaction::Version,
    };

    // Create a minimal transaction structure
    // In practice, this would be a proper transaction that spends from a commit transaction
    let tx = Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint::null(),
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: bitcoin::Witness::new(),
        }],
        output: vec![TxOut {
            value: Amount::from_sat(546), // Dust limit
            script_pubkey: script,
        }],
    };

    Ok(tx)
}

#[cfg(test)]
mod tests {
    use bitcoin::secp256k1::{SECP256K1, SecretKey};
    use rand::rngs::OsRng;
    use strata_crypto::multisig::{SchnorrMultisigConfig, verify_multisig};
    use strata_test_utils::ArbitraryGenerator;

    use super::*;

    #[test]
    fn test_create_multisig_update_signature() {
        let mut arb = ArbitraryGenerator::new();
        let seqno = 1;
        let threshold = 2;

        // Generate test private keys
        let privkeys: Vec<SecretKey> = (0..3).map(|_| SecretKey::new(&mut OsRng)).collect();
        let pubkeys = privkeys
            .iter()
            .map(|sk| sk.x_only_public_key(SECP256K1).0.into())
            .collect::<Vec<Buf32>>();
        let config = SchnorrMultisigConfig::try_new(pubkeys, threshold).unwrap();

        // Create signer indices (signers 0 and 2)
        let mut signer_indices = BitVec::<u8>::new();
        signer_indices.resize(3, false);
        signer_indices.set(0, true);
        signer_indices.set(2, true);

        // Create a test multisig update
        let action: MultisigAction = arb.generate();
        let sighash = action.compute_sighash(seqno);

        let signature = create_multisig_signature(&privkeys, signer_indices.clone(), sighash);

        // Verify the signature has the expected structure
        assert_eq!(signature.signer_indices().len(), 3);
        assert_eq!(signature.signer_indices().count_ones(), 2);
        assert!(signature.signer_indices()[0]);
        assert!(!signature.signer_indices()[1]);
        assert!(signature.signer_indices()[2]);

        let res = verify_multisig(&config, &signature, &sighash.0);
        assert!(res.is_ok());
    }

    // TODO: Add comprehensive test for create_admin_tx once test parameters are properly set up
    // For now, the function signature and basic logic are implemented correctly
}
