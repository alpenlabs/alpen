use bitcoin::Transaction;
use strata_btc_types::{DepositInfo, DepositSpendInfo};
use strata_primitives::l1::OutputRef;

mod checkpoint;
pub mod indexer;
pub mod types;
mod withdrawal_fulfillment;

use checkpoint::parse_valid_checkpoint_envelopes;
use withdrawal_fulfillment::try_parse_tx_as_withdrawal_fulfillment;

use crate::{deposit::deposit_tx::extract_deposit_info, filter::types::TxFilterConfig};

// TODO move all these functions to other modules
/// Parse deposits from [`Transaction`].
fn try_parse_tx_deposit(
    tx: &Transaction,
    filter_conf: &TxFilterConfig,
) -> impl Iterator<Item = DepositInfo> {
    // TODO: Currently only one item is parsed, need to check thoroughly and parse multiple
    extract_deposit_info(tx, &filter_conf.deposit_config).into_iter()
}

/// Parse da blobs from [`Transaction`].
fn extract_da_blobs<'a>(
    _tx: &'a Transaction,
    _filter_conf: &TxFilterConfig,
) -> impl Iterator<Item = impl Iterator<Item = &'a [u8]> + 'a> {
    // TODO: actually implement this when we have da
    std::iter::empty::<std::slice::Iter<'a, &'a [u8]>>().map(|inner| inner.copied())
}

/// Parse transaction and filter out any deposits that have been spent.
fn find_deposit_spends<'tx>(
    tx: &'tx Transaction,
    filter_conf: &'tx TxFilterConfig,
) -> impl Iterator<Item = DepositSpendInfo> + 'tx {
    tx.input.iter().filter_map(|txin| {
        let prevout = OutputRef::new(txin.previous_output.txid, txin.previous_output.vout);
        filter_conf
            .expected_outpoints
            .get(&prevout)
            .map(|config| DepositSpendInfo {
                deposit_idx: config.deposit_idx,
            })
    })
}

#[cfg(test)]
mod test {
    use bitcoin::{
        secp256k1::{Keypair, Secp256k1, SecretKey},
        Amount, ScriptBuf,
    };
    use strata_test_utils_btc::{
        build_test_deposit_script, create_test_deposit_tx, test_taproot_addr,
    };
    use strata_test_utils_l2::gen_params;

    use crate::{filter::try_parse_tx_deposit, utils::test_utils::create_tx_filter_config};

    #[test]
    fn test_parse_deposit_txs() {
        let params = gen_params();
        let (filter_conf, keypair) = create_tx_filter_config(&params);

        let deposit_config = filter_conf.deposit_config.clone();
        let idx = 0xdeadbeef;
        let ee_addr = vec![1u8; 20]; // Example EVM address
        let tapnode_hash = [0u8; 32]; // A dummy tapnode hash. Dummy works because we don't need to
                                      // test takeback at this moment
        let deposit_script =
            build_test_deposit_script(&deposit_config, idx, ee_addr.clone(), &tapnode_hash);

        let tx = create_test_deposit_tx(
            Amount::from_sat(deposit_config.deposit_amount.to_sat()),
            &deposit_config.address.address().script_pubkey(),
            &deposit_script,
            &keypair,
            &tapnode_hash,
        );

        let deposits: Vec<_> = try_parse_tx_deposit(&tx, &filter_conf).collect();
        assert_eq!(deposits.len(), 1, "Should find one deposit transaction");

        assert_eq!(deposits[0].deposit_idx, idx, "deposit idx should match");
        assert_eq!(deposits[0].address, ee_addr, "EE address should match");
        assert_eq!(
            deposits[0].amt, deposit_config.deposit_amount,
            "Deposit amount should match"
        );
    }

    #[test]
    fn test_parse_invalid_deposit_empty_opreturn() {
        let params = gen_params();
        let (filter_conf, keypair) = create_tx_filter_config(&params);

        let deposit_conf = filter_conf.deposit_config.clone();
        let tapnode_hash = [0u8; 32];

        // This won't have magic bytes in script so shouldn't get parsed.
        let tx = create_test_deposit_tx(
            Amount::from_sat(deposit_conf.deposit_amount.to_sat()),
            &test_taproot_addr().address().script_pubkey(),
            &ScriptBuf::new(),
            &keypair,
            &tapnode_hash,
        );

        let deposits: Vec<_> = try_parse_tx_deposit(&tx, &filter_conf).collect();
        assert!(deposits.is_empty(), "Should find no deposit");
    }

    #[test]
    fn test_parse_invalid_deposit_invalid_tapnode_hash() {
        let params = gen_params();
        let (filter_conf, keypair) = create_tx_filter_config(&params);

        let deposit_conf = filter_conf.deposit_config.clone();
        let expected_tapnode_hash = [0u8; 32];
        let ee_addr = vec![1u8; 20]; // Example EVM address
        let idx = 0;

        let mismatching_tapnode_hash = [1u8; 32];
        let deposit_script = build_test_deposit_script(
            &deposit_conf,
            idx,
            ee_addr.clone(),
            &mismatching_tapnode_hash,
        );

        // This won't have magic bytes in script so shouldn't get parsed.
        let tx = create_test_deposit_tx(
            Amount::from_sat(deposit_conf.deposit_amount.to_sat()),
            &test_taproot_addr().address().script_pubkey(),
            &deposit_script,
            &keypair,
            &expected_tapnode_hash,
        );

        let deposits: Vec<_> = try_parse_tx_deposit(&tx, &filter_conf).collect();
        assert!(deposits.is_empty(), "Should find no deposit request");
    }

    #[test]
    fn test_parse_invalid_deposit_invalid_signature() {
        let params = gen_params();
        let (filter_conf, _keypair) = create_tx_filter_config(&params);

        let deposit_config = filter_conf.deposit_config.clone();
        let idx = 0xdeadbeef;
        let ee_addr = vec![1u8; 20]; // Example EVM address
        let tapnode_hash = [0u8; 32]; // A dummy tapnode hash. Dummy works because we don't need to
                                      // test takeback at this moment
        let deposit_script =
            build_test_deposit_script(&deposit_config, idx, ee_addr.clone(), &tapnode_hash);

        let secp = Secp256k1::new();
        // Create a random secret key
        let secret_key = SecretKey::from_slice(&[111u8; 32]).unwrap();
        let invalid_keypair = Keypair::from_secret_key(&secp, &secret_key);
        let tx = create_test_deposit_tx(
            Amount::from_sat(deposit_config.deposit_amount.to_sat()),
            &deposit_config.address.address().script_pubkey(),
            &deposit_script,
            &invalid_keypair,
            &tapnode_hash,
        );

        let deposits: Vec<_> = try_parse_tx_deposit(&tx, &filter_conf).collect();
        assert!(deposits.is_empty(), "Should find no deposit request");
    }
}
