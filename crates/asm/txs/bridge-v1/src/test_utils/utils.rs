use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
use strata_crypto::{EvenPublicKey, EvenSecretKey};

// Helper function to create test operator keys
///
/// # Returns
///
/// - `Vec<EvenSecretKey>` - Private keys for creating test transactions
/// - `Vec<EvenPublicKey>` - MuSig2 public keys for bridge configuration
pub fn create_test_operators(num_operators: usize) -> (Vec<EvenSecretKey>, Vec<EvenPublicKey>) {
    let mut rng = rand::thread_rng();
    let secp = Secp256k1::new();

    // Generate random operator keys
    let operators_privkeys: Vec<EvenSecretKey> = (0..num_operators)
        .map(|_| SecretKey::new(&mut rng).into())
        .collect();

    // Create operator MuSig2 public keys for config
    let operator_pubkeys: Vec<EvenPublicKey> = operators_privkeys
        .iter()
        .map(|sk| EvenPublicKey::from(PublicKey::from_secret_key(&secp, sk)))
        .collect();

    (operators_privkeys, operator_pubkeys)
}
