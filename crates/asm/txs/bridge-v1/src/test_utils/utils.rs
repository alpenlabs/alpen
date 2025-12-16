use bitcoin::{
    Transaction,
    secp256k1::{PublicKey, Secp256k1, SecretKey},
};
use strata_asm_common::TxInputRef;
use strata_crypto::{EvenPublicKey, EvenSecretKey};
use strata_l1_txfmt::{ParseConfig, TagData};

use crate::test_utils::TEST_MAGIC_BYTES;

// Helper function to mutate SPS 50 transaction auxiliary data
pub fn mutate_aux_data(tx: &mut Transaction, new_aux: Vec<u8>) {
    let config = ParseConfig::new(*TEST_MAGIC_BYTES);
    let td = config.try_parse_tx(tx).unwrap();
    let new_td = TagData::new(td.subproto_id(), td.tx_type(), new_aux).unwrap();
    let new_scriptbuf = config.encode_script_buf(&new_td.as_ref()).unwrap();
    tx.output[0].script_pubkey = new_scriptbuf
}

// Helper function to parse transaction
pub fn parse_sps50_tx(tx: &Transaction) -> TxInputRef<'_> {
    let parser = ParseConfig::new(*TEST_MAGIC_BYTES);
    let tag_data = parser.try_parse_tx(tx).expect("Should parse transaction");
    TxInputRef::new(tx, tag_data)
}

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
