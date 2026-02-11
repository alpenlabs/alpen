use bitcoin::{
    Transaction,
    secp256k1::{Message, SECP256K1, SecretKey},
};
use strata_asm_txs_test_utils::create_reveal_transaction_stub;
use strata_crypto::threshold_signature::{IndexedSignature, SignatureSet};
use strata_primitives::buf::Buf32;

use crate::{actions::MultisigAction, parser::SignedPayload};

/// Creates an ECDSA signature with recoverable public key for a message hash.
///
/// Returns a 65-byte signature in the format: recovery_id || r || s
pub fn sign_ecdsa_recoverable(message_hash: &[u8; 32], secret_key: &SecretKey) -> [u8; 65] {
    let message = Message::from_digest_slice(message_hash).expect("32 bytes");
    let sig = SECP256K1.sign_ecdsa_recoverable(&message, secret_key);
    let (recovery_id, compact) = sig.serialize_compact();

    let mut result = [0u8; 65];
    result[0] = recovery_id.to_i32() as u8;
    result[1..65].copy_from_slice(&compact);
    result
}

/// Creates a SignatureSet for any MultisigAction.
///
/// This function generates the required signatures for any administration action
/// (Update or Cancel) by computing the sighash from the action and sequence number,
/// then creating individual ECDSA signatures using the provided private keys.
///
/// # Arguments
/// * `privkeys` - Private keys of all signers in the threshold config
/// * `signer_indices` - Indices of signers participating in this signature
/// * `sighash` - The message hash to sign
///
/// # Returns
/// A SignatureSet that can be used to authorize this action
pub fn create_signature_set(
    privkeys: &[SecretKey],
    signer_indices: &[u8],
    sighash: Buf32,
) -> SignatureSet {
    let signatures: Vec<IndexedSignature> = signer_indices
        .iter()
        .map(|&index| {
            let sig = sign_ecdsa_recoverable(&sighash.0, &privkeys[index as usize]);
            IndexedSignature::new(index, sig)
        })
        .collect();

    SignatureSet::new(signatures).expect("valid signature set")
}

/// Creates a SPS-50 compliant administration transaction with commit-reveal pattern.
///
/// This function creates only the reveal transaction that contains both the action and signatures.
/// The reveal transaction uses the envelope script format to embed the administration payload
/// in a way that's compatible with SPS-50.
///
/// The signed payload (action + signatures) is embedded in the witness envelope, while only
/// the minimal SPS-50 tag (magic bytes, subprotocol ID, tx type) is placed in the OP_RETURN.
///
/// # Arguments
/// * `privkeys` - Private keys of all signers in the threshold config
/// * `signer_indices` - Indices of signers participating in this signature
/// * `action` - The MultisigAction to sign and embed (Update or Cancel)
/// * `seqno` - The sequence number for this operation
///
/// # Returns
/// A Bitcoin transaction that serves as the reveal transaction containing the administration
/// payload
pub fn create_test_admin_tx(
    privkeys: &[SecretKey],
    signer_indices: &[u8],
    action: &MultisigAction,
    seqno: u64,
) -> Transaction {
    // Compute the signature hash and create the signature set
    let sighash = action.compute_sighash(seqno);
    let signature_set = create_signature_set(privkeys, signer_indices, sighash);

    // Create the signed payload (action + signatures) for the envelope
    let signed_payload = SignedPayload::new(seqno, action.clone(), signature_set);
    let envelope_payload = borsh::to_vec(&signed_payload).expect("borsh serialization failed");

    // Create a minimal reveal transaction structure
    // This is a simplified version - in practice, this would be created as part of
    // a proper commit-reveal transaction pair using the btcio writer infrastructure
    create_reveal_transaction_stub(envelope_payload, &action.tag())
}

#[cfg(test)]
mod tests {
    use std::num::NonZero;

    use bitcoin::secp256k1::PublicKey;
    use rand::rngs::OsRng;
    use strata_asm_common::TxInputRef;
    use strata_asm_txs_test_utils::TEST_MAGIC_BYTES;
    use strata_crypto::{
        keys::compressed::CompressedPublicKey,
        threshold_signature::{ThresholdConfig, verify_threshold_signatures},
    };
    use strata_l1_txfmt::ParseConfig;
    use strata_test_utils::ArbitraryGenerator;

    use super::*;
    use crate::parser::parse_tx;

    #[test]
    fn test_create_signature_set() {
        let mut arb = ArbitraryGenerator::new();
        let seqno = 1;
        let threshold = NonZero::new(2).unwrap();

        // Generate test private keys
        let privkeys: Vec<SecretKey> = (0..3).map(|_| SecretKey::new(&mut OsRng)).collect();
        let pubkeys: Vec<CompressedPublicKey> = privkeys
            .iter()
            .map(|sk| CompressedPublicKey::from(PublicKey::from_secret_key(SECP256K1, sk)))
            .collect();
        let config = ThresholdConfig::try_new(pubkeys, threshold).unwrap();

        // Create signer indices (signers 0 and 2)
        let signer_indices = [0u8, 2u8];

        // Create a test multisig action
        let action: MultisigAction = arb.generate();
        let sighash = action.compute_sighash(seqno);

        let signature_set = create_signature_set(&privkeys, &signer_indices, sighash);

        // Verify the signature set has the expected structure
        assert_eq!(signature_set.len(), 2);
        let indices: Vec<u8> = signature_set.indices().collect();
        assert_eq!(indices, vec![0, 2]);

        // Verify the signatures
        let res = verify_threshold_signatures(&config, signature_set.signatures(), &sighash.0);
        assert!(res.is_ok());
    }

    #[test]
    fn test_admin_tx() {
        let mut arb = ArbitraryGenerator::new();
        let seqno = 1;
        let threshold = NonZero::new(2).unwrap();

        // Generate test private keys
        let privkeys: Vec<SecretKey> = (0..3).map(|_| SecretKey::new(&mut OsRng)).collect();
        let pubkeys: Vec<CompressedPublicKey> = privkeys
            .iter()
            .map(|sk| CompressedPublicKey::from(PublicKey::from_secret_key(SECP256K1, sk)))
            .collect();
        let config = ThresholdConfig::try_new(pubkeys, threshold).unwrap();

        // Create signer indices (signers 0 and 2)
        let signer_indices = [0u8, 2u8];

        let action: MultisigAction = arb.generate();
        let tx = create_test_admin_tx(&privkeys, &signer_indices, &action, seqno);
        let tag_data_ref = ParseConfig::new(TEST_MAGIC_BYTES)
            .try_parse_tx(&tx)
            .unwrap();
        let tx_input = TxInputRef::new(&tx, tag_data_ref);

        let parsed = parse_tx(&tx_input).unwrap();
        assert_eq!(action, parsed.action);

        // Verify the signatures
        let res = verify_threshold_signatures(
            &config,
            &parsed.signatures.signatures(),
            &action.compute_sighash(seqno).0,
        );
        assert!(res.is_ok());
    }
}
