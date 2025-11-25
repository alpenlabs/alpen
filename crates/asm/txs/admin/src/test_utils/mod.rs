use bitcoin::{
    Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness, XOnlyPublicKey,
    absolute::LockTime,
    blockdata::script,
    key::UntweakedKeypair,
    opcodes::{
        OP_FALSE,
        all::{OP_CHECKMULTISIG, OP_ENDIF, OP_IF},
    },
    script::PushBytesBuf,
    secp256k1::{Message, SECP256K1, SecretKey, schnorr::Signature},
    taproot::{LeafVersion, TaprootBuilder},
    transaction::Version,
};
use rand::{RngCore, rngs::OsRng};
use strata_crypto::threshold_signing::{IndexedSignature, SignatureSet};
use strata_primitives::buf::Buf32;

pub(crate) const TEST_MAGIC_BYTES: &[u8; 4] = b"ALPN";

use crate::{
    actions::MultisigAction,
    constants::ADMINISTRATION_SUBPROTOCOL_ID,
    parser::SignedPayload,
};

/// Creates an ECDSA recoverable signature for a message hash.
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
    let signed_payload = SignedPayload::new(action.clone(), signature_set);
    let envelope_payload = borsh::to_vec(&signed_payload).expect("borsh serialization failed");

    // Create the minimal SPS-50 tag for OP_RETURN (no aux data needed)
    // Format: [MAGIC_BYTES][SUBPROTOCOL_ID][TX_TYPE]
    let mut tagged_payload = Vec::new();
    tagged_payload.extend_from_slice(TEST_MAGIC_BYTES); // 4 bytes magic
    tagged_payload.extend_from_slice(&ADMINISTRATION_SUBPROTOCOL_ID.to_be_bytes()); // 1 byte subprotocol ID
    tagged_payload.extend_from_slice(&[action.tx_type()]); // 1 byte TxType

    // Create a minimal reveal transaction structure
    // This is a simplified version - in practice, this would be created as part of
    // a proper commit-reveal transaction pair using the btcio writer infrastructure
    create_reveal_transaction_stub(envelope_payload, tagged_payload)
}

/// Creates a stub reveal transaction containing the envelope script.
/// This is a simplified implementation for testing purposes.
fn create_reveal_transaction_stub(
    envelope_payload: Vec<u8>,
    sps50_tagged_payload: Vec<u8>,
) -> Transaction {
    // Create commit key
    let mut rand_bytes = [0; 32];
    OsRng.fill_bytes(&mut rand_bytes);
    let key_pair = UntweakedKeypair::from_seckey_slice(SECP256K1, &rand_bytes).unwrap();
    let public_key = XOnlyPublicKey::from_keypair(&key_pair).0;

    // Start creating envelope content
    let reveal_script = build_reveal_script(&public_key, &envelope_payload);

    // Create spend info for tapscript
    let taproot_spend_info = TaprootBuilder::new()
        .add_leaf(0, reveal_script.clone())
        .unwrap()
        .finalize(SECP256K1, public_key)
        .expect("Could not build taproot spend info");

    let signature = Signature::from_slice(&[0u8; 64]).unwrap();
    let mut witness = Witness::new();
    witness.push(signature.as_ref());
    witness.push(reveal_script.clone());
    witness.push(
        taproot_spend_info
            .control_block(&(reveal_script, LeafVersion::TapScript))
            .expect("Could not create control block")
            .serialize(),
    );

    Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint::null(),
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness,
        }],
        output: vec![TxOut {
            value: Amount::ZERO,
            script_pubkey: ScriptBuf::new_op_return(
                PushBytesBuf::try_from(sps50_tagged_payload).unwrap(),
            ),
        }],
    }
}

/// Builds reveal script such that it contains opcodes for verifying the internal key as well as the
/// envelope block
fn build_reveal_script(taproot_public_key: &XOnlyPublicKey, payload: &[u8]) -> ScriptBuf {
    let mut script_bytes = script::Builder::new()
        .push_x_only_key(taproot_public_key)
        .push_opcode(OP_CHECKMULTISIG)
        .into_script()
        .into_bytes();
    let script = build_envelope_script(payload);
    script_bytes.extend(script.into_bytes());
    ScriptBuf::from(script_bytes)
}

fn build_envelope_script(payload: &[u8]) -> ScriptBuf {
    let mut builder = script::Builder::new()
        .push_opcode(OP_FALSE)
        .push_opcode(OP_IF);

    // Insert actual data
    for chunk in payload.chunks(520) {
        builder = builder.push_slice(PushBytesBuf::try_from(chunk.to_vec()).unwrap());
    }
    builder = builder.push_opcode(OP_ENDIF);
    builder.into_script()
}

#[cfg(test)]
mod tests {
    use bitcoin::secp256k1::{PublicKey, Secp256k1};
    use rand::rngs::OsRng;
    use strata_asm_common::TxInputRef;
    use strata_crypto::threshold_signing::{
        verify_threshold_signatures, CompressedPublicKey, ThresholdConfig,
    };
    use strata_l1_txfmt::ParseConfig;
    use strata_test_utils::ArbitraryGenerator;

    use super::*;
    use crate::parser::parse_tx;

    #[test]
    fn test_create_signature_set() {
        let mut arb = ArbitraryGenerator::new();
        let seqno = 1;
        let threshold = 2;
        let secp = Secp256k1::new();

        // Generate test private keys
        let privkeys: Vec<SecretKey> = (0..3).map(|_| SecretKey::new(&mut OsRng)).collect();
        let pubkeys: Vec<CompressedPublicKey> = privkeys
            .iter()
            .map(|sk| CompressedPublicKey::from(PublicKey::from_secret_key(&secp, sk)))
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
        let res = verify_threshold_signatures(&config, &signature_set, &sighash.0);
        assert!(res.is_ok());
    }

    #[test]
    fn test_admin_tx() {
        let mut arb = ArbitraryGenerator::new();
        let seqno = 1;
        let threshold = 2;
        let secp = Secp256k1::new();

        // Generate test private keys
        let privkeys: Vec<SecretKey> = (0..3).map(|_| SecretKey::new(&mut OsRng)).collect();
        let pubkeys: Vec<CompressedPublicKey> = privkeys
            .iter()
            .map(|sk| CompressedPublicKey::from(PublicKey::from_secret_key(&secp, sk)))
            .collect();
        let config = ThresholdConfig::try_new(pubkeys, threshold).unwrap();

        // Create signer indices (signers 0 and 2)
        let signer_indices = [0u8, 2u8];

        let action: MultisigAction = arb.generate();
        let tx = create_test_admin_tx(&privkeys, &signer_indices, &action, seqno);
        let tag_data_ref = ParseConfig::new(*TEST_MAGIC_BYTES)
            .try_parse_tx(&tx)
            .unwrap();
        let tx_input = TxInputRef::new(&tx, tag_data_ref);

        let (p_action, sig) = parse_tx(&tx_input).unwrap();
        assert_eq!(action, p_action);

        // Verify the signatures
        let res = verify_threshold_signatures(&config, &sig, &action.compute_sighash(seqno).0);
        assert!(res.is_ok());
    }
}
