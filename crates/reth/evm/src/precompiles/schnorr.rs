use revm::precompile::{utilities::right_pad, PrecompileOutput, PrecompileResult};
use revm_primitives::Bytes;
use strata_crypto::verify_schnorr_sig;
use strata_primitives::buf::{Buf32, Buf64};

/// Internal representation of parsed Schnorr input bytes.
struct SchnorrInput {
    /// 32 Bytes: Public key
    public_key: Buf32,
    /// 32 Bytes: Message hash
    message_hash: Buf32,
    /// 64 Bytes: Schnorr Signature
    signature: Buf64,
}

fn parse_schnorr_input(input: &Bytes) -> SchnorrInput {
    let input = right_pad::<128>(input);

    SchnorrInput {
        public_key: Buf32::new(input[0..32].try_into().unwrap()),
        message_hash: Buf32::new(input[32..64].try_into().unwrap()),
        signature: Buf64::new(input[64..128].try_into().unwrap()),
    }
}

pub(crate) fn verify_schnorr_precompile(input: &Bytes, _gas_limit: u64) -> PrecompileResult {
    let schnorr_input = parse_schnorr_input(input);

    let result = verify_schnorr_sig(
        &schnorr_input.signature,
        &schnorr_input.message_hash,
        &schnorr_input.public_key,
    );
    let verification_byte = Bytes::from([result as u8]);

    // currently we can use [ecrecover hack](https://hackmd.io/@nZ-twauPRISEa6G9zg3XRw/SyjJzSLt9)
    // which costs around ~3000 gas.
    // setting it as 0, as this requires further discussion
    let gas_cost = 0;

    Ok(PrecompileOutput::new(gas_cost, verification_byte))
}

#[cfg(test)]
mod tests {
    use secp256k1::{Keypair, SecretKey, SECP256K1};
    use strata_crypto::sign_schnorr_sig;
    use strata_primitives::buf::{Buf32, Buf64};

    use super::*;

    /// Generates a valid input where the signature ends in zeroes.
    fn generate_valid_input() -> Bytes {
        let secret_key = Buf32::new([1u8; 32]);
        let message_hash = Buf32::new([1u8; 32]);
        let schnorr_sig = sign_schnorr_sig(&message_hash, &secret_key);
        let keypair = Keypair::from_secret_key(SECP256K1, &SecretKey::from(secret_key));
        let public_key = keypair.x_only_public_key().0;

        let mut input = Vec::new();
        input.extend_from_slice(&public_key.serialize());
        input.extend_from_slice(message_hash.as_ref());
        input.extend_from_slice(schnorr_sig.as_ref());
        Bytes::from(input)
    }

    /// Generates an input where the signature doesn't end in zeroes.
    fn generate_invalid_input() -> Bytes {
        let public_key = Buf32::new([1u8; 32]);
        let message_hash = Buf32::new([2u8; 32]);
        let signature = Buf64::new([3u8; 64]); // No zero at the end

        let mut input = Vec::new();
        input.extend_from_slice(public_key.as_ref());
        input.extend_from_slice(message_hash.as_ref());
        input.extend_from_slice(signature.as_ref());
        Bytes::from(input)
    }

    #[test]
    fn test_signature_ends_with_zero() {
        let input = generate_valid_input();
        let result = verify_schnorr_precompile(&input, 0).unwrap();

        assert_eq!(
            result.bytes,
            Bytes::from([1]),
            "Expected valid signature with trailing zero to return 1"
        );
    }

    #[test]
    fn test_signature_does_not_end_with_zero() {
        let input = generate_invalid_input();
        let result = verify_schnorr_precompile(&input, 0).unwrap();

        assert_eq!(
            result.bytes,
            Bytes::from([0]),
            "Expected invalid signature without trailing zero to return 0"
        );
    }

    #[test]
    fn test_input_with_wrong_length() {
        let input = Bytes::from(vec![1u8; 100]); // Not 128 bytes
        let result = verify_schnorr_precompile(&input, 0).unwrap();

        assert_eq!(result.bytes, Bytes::from([0]));
    }
}
