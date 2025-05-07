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
    use bitcoin::{Amount, ScriptBuf};
    use strata_primitives::l1::BitcoinAmount;
    use strata_test_utils::{
        bitcoin::{
            build_test_deposit_request_script, build_test_deposit_script, create_test_deposit_tx,
            test_taproot_addr,
        },
        l2::gen_params,
    };

    use crate::{
        filter::{extract_deposit_requests, try_parse_tx_deposit},
        utils::test_utils::create_tx_filter_config,
    };

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
    fn test_parse_deposit_request() {
        let params = gen_params();
        let (filter_conf, keypair) = create_tx_filter_config(&params);
        let mut deposit_conf = filter_conf.deposit_config.clone();

        let extra_amt = 10000;
        deposit_conf.deposit_amount += extra_amt;
        let dest_addr = vec![2u8; 20]; // Example EVM address
        let dummy_block = [0u8; 32]; // Example dummy block
        let deposit_request_script = build_test_deposit_request_script(
            deposit_conf.magic_bytes.clone(),
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

    /// Tests parsing deposits which are invalid, i.e won't parse.
    #[test]
    fn test_parse_invalid_deposit() {
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
        assert!(deposits.is_empty(), "Should find no deposit request");
    }
}
