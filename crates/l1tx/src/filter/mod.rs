use bitcoin::Transaction;
use strata_primitives::l1::{DepositInfo, DepositRequestInfo, DepositSpendInfo, OutputRef};

mod checkpoint;
pub mod indexer;
pub mod types;
mod withdrawal_fulfillment;

use checkpoint::parse_valid_checkpoint_envelopes;
use withdrawal_fulfillment::try_parse_tx_as_withdrawal_fulfillment;

use crate::{
    deposit::{deposit_request::extract_deposit_request_info, deposit_tx::extract_deposit_info},
    filter::types::TxFilterConfig,
};

// TODO move all these functions to other modules

fn extract_deposit_requests(
    tx: &Transaction,
    filter_conf: &TxFilterConfig,
) -> impl Iterator<Item = DepositRequestInfo> {
    // TODO: Currently only one item is parsed, need to check thoroughly and parse multiple
    extract_deposit_request_info(tx, &filter_conf.deposit_config).into_iter()
}

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
    use strata_primitives::l1::BitcoinAmount;
    use strata_test_utils_btc::{
        build_test_deposit_request_script, build_test_deposit_script, create_test_deposit_tx,
        test_taproot_addr,
    };
    use strata_test_utils_l2::gen_params;

    use crate::{
        filter::{extract_deposit_requests, try_parse_tx_deposit},
        utils::test_utils::create_tx_filter_config,
    };

    #[test]
    fn test_parse_deposit_request() {
        let params = gen_params();
        let (filter_conf, keypair) = create_tx_filter_config(&params);
        let mut deposit_conf = filter_conf.deposit_config.clone();

        let extra_amt = 10000;
        deposit_conf.deposit_amount += extra_amt;
        let dest_addr = vec![2u8; 20]; // Example EVM address
        let dummy_block = [0u8; 32]; // Example dummy block
        let deposit_request_script = build_test_deposit_request_script(
            deposit_conf.magic_bytes.to_vec(),
            dummy_block.to_vec(),
            dest_addr.clone(),
        );

        let tapnode_hash = [0u8; 32];

        let tx = create_test_deposit_tx(
            Amount::from_sat(deposit_conf.deposit_amount), // Any amount
            &deposit_conf.address.address().script_pubkey(),
            &deposit_request_script,
            &keypair,
            &tapnode_hash,
        );

        let deposit_reqs: Vec<_> = extract_deposit_requests(&tx, &filter_conf).collect();
        assert_eq!(deposit_reqs.len(), 1, "Should find one deposit request");

        assert_eq!(
            deposit_reqs[0].address, dest_addr,
            "EE address should match"
        );
        assert_eq!(
            deposit_reqs[0].take_back_leaf_hash, dummy_block,
            "Control block should match"
        );
    }

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
            Amount::from_sat(deposit_config.deposit_amount),
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
            deposits[0].amt,
            BitcoinAmount::from_sat(deposit_config.deposit_amount),
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
            Amount::from_sat(deposit_conf.deposit_amount),
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
            Amount::from_sat(deposit_conf.deposit_amount),
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
            Amount::from_sat(deposit_config.deposit_amount),
            &deposit_config.address.address().script_pubkey(),
            &deposit_script,
            &invalid_keypair,
            &tapnode_hash,
        );

        let deposits: Vec<_> = try_parse_tx_deposit(&tx, &filter_conf).collect();
        assert!(deposits.is_empty(), "Should find no deposit request");
    }

    // REVIEW: removing the test vector because it is using "strata" as magic bytes, but we now
    // limit it to 4 bytes // Set up test vector and decode transaction
    // #[test]
    // fn test_deposit_tx_with_test_vector() {
    //     let txraw =
    //     "0200000000010186e7cee076c18d5d33a642ef689027265be442bce68294a335d8665e4eb81ab10000000000fdffffff0200e1f505000000002251209accbdba14ffa7ca9d636ec63beee4c00c9b9504e4fc4f71a779491907ff1ef10000000000000000486a467374726174610000000070997970c51812dc3a010c7d01b50e0d17dc79c8eb9ee3797b83d854f724f2ab625938deea929282a5330927a0e8ae994b9b2b240000000005f767a0014101a54bdc1dd43416bd0618969eb040735fbbfab1103a6c55a7a80f28e5aaa9c0f57d483dcc84d68d4add23515560c743d789b1a0dd596db40b3bd70407185da20100000000"
    //     ;
    //     let txbytes = Vec::from_hex(txraw).unwrap();
    //     let tx: Transaction = deserialize(&txbytes).expect("failed to deserialize tx");

    //     // The operator pubkeys that has signed the above tx.
    //     let op_pubkeys: Vec<_> = vec![
    //         Vec::from_hex("b49092f76d06f8002e0b7f1c63b5058db23fd4465b4f6954b53e1f352a04754d"),
    //         Vec::from_hex("1e62d54af30569fd7269c14b6766f74d85ea00c911c4e1a423d4ba2ae4c34dc4"),
    //         Vec::from_hex("a4d869ccd09c470f8f86d3f1b0997fa2695933aaea001875b9db145ae9c1f4ba"),
    //     ]
    //     .into_iter()
    //     .map(|x| Buf32::try_from_slice(&x.unwrap()).unwrap())
    //     .map(|wpk| OperatorPubkeys::new(wpk, wpk)) // Just have same wallet/signing keys
    //     .collect();

    //     // Configure and update rollup params by setting the custom operator keys
    //     let mut params = gen_params();

    //     params.rollup.operator_config = OperatorConfig::Static(op_pubkeys);

    //     // Derive filter config and update magic bytes and deposit amount that the above
    // transaction     // has.
    //     let mut filterconf = TxFilterConfig::derive_from(params.rollup()).unwrap();
    //     filterconf.deposit_config.magic_bytes = "strata".bytes().collect();
    //     filterconf.deposit_config.deposit_amount = 100_000_000;

    //     let deposits: Vec<_> = try_parse_tx_deposit(&tx, &filterconf).collect();
    //     assert_eq!(deposits.len(), 1, "Should find one deposit transaction");

    //     // The transaction contains the following ee address in opreturn, check if it is parsed.
    //     let exp_address = vec![
    //         0x70, 0x99, 0x79, 0x70, 0xc5, 0x18, 0x12, 0xdc, 0x3a, 0x01, 0x0c, 0x7d, 0x01, 0xb5,
    //         0x0e, 0x0d, 0x17, 0xdc, 0x79, 0xc8,
    //     ];

    //     assert_eq!(deposits[0].deposit_idx, 0, "deposit idx should match");
    //     assert_eq!(deposits[0].address, exp_address, "EE address should match");
    //     assert_eq!(
    //         deposits[0].amt,
    //         BitcoinAmount::from_sat(filterconf.deposit_config.deposit_amount),
    //         "Deposit amount should match"
    //     );

    //     // Now tamper the witness of the transaction, to check if the validation passes. It
    // should     // not.
    //     let mut tx = tx.clone();
    //     let mut witness = tx.input[0].witness.to_vec();
    //     witness[0][0] ^= 255; // flip the bits
    //     tx.input[0].witness = Witness::from_slice(&witness);
    //     let deposits: Vec<_> = try_parse_tx_deposit(&tx, &filterconf).collect();
    //     assert_eq!(deposits.len(), 0, "Should not find any deposit transaction");
    // }
}
