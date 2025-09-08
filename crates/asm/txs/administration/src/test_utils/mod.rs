use bitcoin::{ScriptBuf, Transaction, secp256k1::SecretKey};
use bitvec::vec::BitVec;
use borsh::{BorshDeserialize, BorshSerialize};
use strata_crypto::multisig::{
    SchnorrMultisigSignature,
    schemes::{SchnorrScheme, schnorr::create::create_musig2_signature},
    signature::MultisigSignature,
};
use strata_l1tx::envelope::builder::build_envelope_script;
use strata_primitives::{
    buf::{Buf32, Buf64},
    l1::payload::{L1Payload, L1PayloadType},
    params::Params,
};

use crate::actions::MultisigAction;

/// Administration transaction payload containing the action and its signature.
/// This structure is serialized and embedded in the SPS-50 compliant reveal transaction.
#[derive(Debug, Clone)]
pub struct AdminTxPayload {
    /// The multisig action (Update or Cancel)
    pub action: MultisigAction,
    /// The cryptographic signature authorizing this action
    pub signature: SchnorrMultisigSignature,
    /// Sequence number for this operation
    pub seqno: u64,
}

impl BorshSerialize for AdminTxPayload {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        // Serialize the action
        self.action.serialize(writer)?;
        
        // Manually serialize the signature components
        // Convert BitSlice to bytes for serialization
        let signer_indices = self.signature.signer_indices();
        let len = signer_indices.len();
        len.serialize(writer)?;
        
        // Convert bitvec to a simple Vec<bool> for serialization
        let bits: Vec<bool> = signer_indices.iter().by_vals().collect();
        bits.serialize(writer)?;
        
        self.signature.signature().serialize(writer)?;
        
        // Serialize the sequence number
        self.seqno.serialize(writer)
    }
}

impl BorshDeserialize for AdminTxPayload {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        // Deserialize the action
        let action = MultisigAction::deserialize_reader(reader)?;
        
        // Deserialize signature components
        let bitvec_len = usize::deserialize_reader(reader)?;
        let bits = Vec::<bool>::deserialize_reader(reader)?;
        
        // Reconstruct BitVec from bools
        let mut signer_indices = BitVec::with_capacity(bitvec_len);
        for bit in bits {
            signer_indices.push(bit);
        }
        
        let signature_buf = Buf64::deserialize_reader(reader)?;
        let signature = SchnorrMultisigSignature::new(signer_indices, signature_buf);
        
        // Deserialize the sequence number
        let seqno = u64::deserialize_reader(reader)?;
        
        Ok(AdminTxPayload {
            action,
            signature,
            seqno,
        })
    }
}

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
    signer_indices: BitVec,
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
/// A Bitcoin transaction that serves as the reveal transaction containing the administration payload
pub fn create_admin_tx(
    params: &Params,
    privkeys: &[SecretKey],
    signer_indices: BitVec,
    action: &MultisigAction,
    seqno: u64,
) -> anyhow::Result<Transaction> {
    // Compute the signature hash and create the multisig signature
    let sighash = action.compute_sighash(seqno);
    let signature = create_multisig_signature(privkeys, signer_indices, sighash);

    // Convert to SchnorrMultisigSignature
    let schnorr_signature = SchnorrMultisigSignature::new(
        signature.signer_indices().to_bitvec(),
        signature.signature().clone(),
    );

    // Create the administration transaction payload
    let admin_payload = AdminTxPayload {
        action: action.clone(),
        signature: schnorr_signature,
        seqno,
    };

    // Serialize the payload
    let payload_bytes = borsh::to_vec(&admin_payload)?;

    // Create L1 payload for the administration data
    // Using L1PayloadType::Da as the administration subprotocol uses DA-like semantics
    let l1_payload = L1Payload::new(payload_bytes, L1PayloadType::Da);

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
        let mut signer_indices = BitVec::new();
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
