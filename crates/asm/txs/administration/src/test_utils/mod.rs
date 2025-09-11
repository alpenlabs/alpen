use anyhow::anyhow;
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
    secp256k1::{SECP256K1, schnorr::Signature},
    taproot::{LeafVersion, TaprootBuilder},
    transaction::Version,
};
use bitvec::vec::BitVec;
use rand::{RngCore, rngs::OsRng};
use strata_crypto::{
    EvenSecretKey,
    multisig::{
        schemes::{SchnorrScheme, schnorr::create::create_musig2_signature},
        signature::MultisigSignature,
    },
};
use strata_primitives::buf::{Buf32, Buf64};

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
    privkeys: &[EvenSecretKey],
    signer_indices: BitVec<u8>,
    sighash: Buf32,
) -> MultisigSignature<SchnorrScheme> {
    // Extract only the private keys for signers indicated by signer_indices
    let selected_privkeys: Vec<EvenSecretKey> = signer_indices
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
pub fn create_test_admin_tx(
    privkeys: &[EvenSecretKey],
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

    let signer_indices_bytes = signature.signer_indices().to_bitvec().into_vec();
    aux_data.extend_from_slice(&signer_indices_bytes); // variable length bitset as bytes

    // Create the complete SPS-50 tagged payload
    // Format: [MAGIC_BYTES][SUBPROTOCOL_ID][TX_TYPE][AUX_DATA]
    let mut tagged_payload = Vec::new();
    tagged_payload.extend_from_slice(TEST_MAGIC_BYTES); // 4 bytes magic
    tagged_payload.extend_from_slice(&ADMINISTRATION_SUBPROTOCOL_ID.to_be_bytes()); // 1 byte subprotocol ID
    tagged_payload.extend_from_slice(&[action.tx_type()]); // 1 byte TxType
    tagged_payload.extend_from_slice(&aux_data); // auxiliary data

    let action_payload = borsh::to_vec(action)?;

    // Create a minimal reveal transaction structure
    // This is a simplified version - in practice, this would be created as part of
    // a proper commit-reveal transaction pair using the btcio writer infrastructure
    let reveal_tx = create_reveal_transaction_stub(action_payload, tagged_payload)?;

    Ok(reveal_tx)
}

/// Creates a stub reveal transaction containing the envelope script.
/// This is a simplified implementation for testing purposes.
fn create_reveal_transaction_stub(
    envelope_payload: Vec<u8>,
    sps50_tagged_payload: Vec<u8>,
) -> anyhow::Result<Transaction> {
    // Create commit key
    let mut rand_bytes = [0; 32];
    OsRng.fill_bytes(&mut rand_bytes);
    let key_pair = UntweakedKeypair::from_seckey_slice(SECP256K1, &rand_bytes)?;
    let public_key = XOnlyPublicKey::from_keypair(&key_pair).0;

    // Start creating envelope content
    let reveal_script = build_reveal_script(&public_key, &envelope_payload)?;

    // Create spend info for tapscript
    let taproot_spend_info = TaprootBuilder::new()
        .add_leaf(0, reveal_script.clone())?
        .finalize(SECP256K1, public_key)
        .map_err(|_| anyhow!("Could not build taproot spend info"))?;

    let signature = Signature::from_slice(&[0u8; 64]).unwrap();
    let mut witness = Witness::new();
    witness.push(signature.as_ref());
    witness.push(reveal_script.clone());
    witness.push(
        taproot_spend_info
            .control_block(&(reveal_script, LeafVersion::TapScript))
            .ok_or(anyhow!("Could not create control block"))?
            .serialize(),
    );

    let tx = Transaction {
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
    };

    Ok(tx)
}

/// Builds reveal script such that it contains opcodes for verifying the internal key as well as the
/// envelope block
fn build_reveal_script(
    taproot_public_key: &XOnlyPublicKey,
    payload: &[u8],
) -> Result<ScriptBuf, anyhow::Error> {
    let mut script_bytes = script::Builder::new()
        .push_x_only_key(taproot_public_key)
        .push_opcode(OP_CHECKMULTISIG)
        .into_script()
        .into_bytes();
    let script = build_envelope_script(payload)?;
    script_bytes.extend(script.into_bytes());
    Ok(ScriptBuf::from(script_bytes))
}

fn build_envelope_script(payload: &[u8]) -> anyhow::Result<ScriptBuf> {
    let mut builder = script::Builder::new()
        .push_opcode(OP_FALSE)
        .push_opcode(OP_IF);

    // Insert actual data
    for chunk in payload.chunks(520) {
        builder = builder.push_slice(PushBytesBuf::try_from(chunk.to_vec())?);
    }
    builder = builder.push_opcode(OP_ENDIF);
    Ok(builder.into_script())
}

#[cfg(test)]
mod tests {
    use bitcoin::secp256k1::{SECP256K1, SecretKey};
    use rand::rngs::OsRng;
    use strata_asm_common::TxInputRef;
    use strata_crypto::multisig::{SchnorrMultisigConfig, verify_multisig};
    use strata_l1_txfmt::ParseConfig;
    use strata_test_utils::ArbitraryGenerator;

    use super::*;
    use crate::parser::parse_tx_multisig_action_and_vote;

    #[test]
    fn test_create_multisig_update_signature() {
        let mut arb = ArbitraryGenerator::new();
        let seqno = 1;
        let threshold = 2;

        // Generate test private keys
        let privkeys: Vec<EvenSecretKey> =
            (0..3).map(|_| SecretKey::new(&mut OsRng).into()).collect();
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

    #[test]
    fn test_admin_tx() {
        let mut arb = ArbitraryGenerator::new();
        let seqno = 1;
        let threshold = 2;

        // Generate test private keys
        // Create signer indices (signers 0 and 2)
        let privkeys: Vec<EvenSecretKey> =
            (0..3).map(|_| SecretKey::new(&mut OsRng).into()).collect();
        let mut signer_indices = BitVec::<u8>::new();
        signer_indices.resize(3, false);
        signer_indices.set(0, true);
        signer_indices.set(2, true);

        let action: MultisigAction = arb.generate();
        let tx = create_test_admin_tx(&privkeys, signer_indices, &action, seqno).unwrap();
        let tag_data_ref = ParseConfig::new(*TEST_MAGIC_BYTES)
            .try_parse_tx(&tx)
            .unwrap();
        let tx_input = TxInputRef::new(&tx, tag_data_ref);

        let (p_action, sig) = parse_tx_multisig_action_and_vote(&tx_input).unwrap();
        assert_eq!(action, p_action);

        let pubkeys = privkeys
            .iter()
            .map(|sk| sk.x_only_public_key(SECP256K1).0.into())
            .collect::<Vec<Buf32>>();
        let config = SchnorrMultisigConfig::try_new(pubkeys, threshold).unwrap();

        let res = verify_multisig(&config, &sig, &action.compute_sighash(seqno).0);
        assert!(res.is_ok());
    }
}
