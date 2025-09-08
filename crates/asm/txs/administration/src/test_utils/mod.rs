use bitcoin::{Transaction, secp256k1::SecretKey};
use bitvec::vec::BitVec;
use strata_crypto::multisig::{
    schemes::{SchnorrScheme, schnorr::create::create_musig2_signature},
    signature::MultisigSignature,
};
use strata_primitives::buf::{Buf32, Buf64};

use crate::actions::MultisigAction;

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

// pub fn create_admin_tx(
//     privkeys: &[SecretKey],
//     signer_indices: BitVec,
//     action: &MultisigAction,
//     seqno: u64,
// ) -> Transaction {
//     let sighash = action.compute_sighash(seqno);
//     let sig = create_multisig_signature(privkeys, signer_indices, sighash)
// }

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
}
