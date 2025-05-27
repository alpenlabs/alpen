use bitcoin::{ScriptBuf, Transaction};
use strata_primitives::{
    buf::Buf32,
    l1::{BitcoinAmount, WithdrawalFulfillmentInfo},
};
use tracing::{debug, error};

use crate::filter::types::TxFilterConfig;

/// Parse transaction and search for a Withdrawal Fulfillment transaction to an expected address.
pub(crate) fn try_parse_tx_as_withdrawal_fulfillment(
    tx: &Transaction,
    filter_conf: &TxFilterConfig,
) -> Option<WithdrawalFulfillmentInfo> {
    // 1. Check this is of correct structure
    let frontpayment_txout = tx.output.first()?;
    let metadata_txout = tx.output.get(1)?;
    let txid: Buf32 = tx.compute_txid().into();

    metadata_txout.script_pubkey.is_op_return().then_some(())?;

    // 2. Ensure correct OP_RETURN data and check it has expected deposit index.

    // FIXME this just takes the first 4 bytes of deposit config magic bytes
    let magic_config = &filter_conf.deposit_config.magic_bytes;
    let Ok(magic) = <[u8; 4]>::try_from(&magic_config[..4]) else {
        error!("malformed magic bytes in deposit config");
        return None;
    };

    let (op_idx, dep_idx, deposit_txid_bytes) =
        parse_opreturn_metadata(&metadata_txout.script_pubkey, &magic)?;

    let exp_ful = filter_conf.expected_withdrawal_fulfillments.get(&dep_idx)?;
    //eprintln!("exp ful {exp_ful:?}");

    if exp_ful.operator_idx != op_idx {
        //eprintln!("wrong operator");
        debug!(?txid, "Deposit index matches but operator_idx does not");
        return None;
    }

    // 3. Ensure deposit txid in metadata is correct
    if exp_ful.deposit_txid != deposit_txid_bytes {
        //eprintln!("wrong deposit txid");
        debug!(
            ?txid,
            "Deposit index and operator index matches but deposit txid does not"
        );
        return None;
    }

    // 4. Check if it is spent to expected destination.
    if frontpayment_txout.script_pubkey != *exp_ful.destination.inner() {
        //eprintln!("wrong spk");
        debug!(
            ?txid,
            "Deposit index and operator index matches but script_pubkey does not"
        );
        return None;
    }

    // 5. Ensure amount is equal to the expected amount
    let actual_amount_sats = frontpayment_txout.value.to_sat();
    if actual_amount_sats < exp_ful.min_amount.to_sat() {
        //eprintln!("wrong amt {actual_amount_sats} {}", exp_ful.min_amount);
        debug!(
            ?txid,
            "Deposit index and script_pubkey match but the amount does not"
        );
        return None;
    }

    Some(WithdrawalFulfillmentInfo {
        deposit_idx: exp_ful.deposit_idx,
        operator_idx: exp_ful.operator_idx,
        amt: BitcoinAmount::from_sat(actual_amount_sats),
        txid: tx.compute_txid().into(),
    })
}

fn parse_opreturn_metadata(
    script_buf: &ScriptBuf,
    magic: &[u8; 4],
) -> Option<(u32, u32, [u8; 32])> {
    // FIXME this needs to ensure that it's actually an OP_RETURN
    let data = extract_op_return_data(script_buf)?;

    // 4 bytes magic + 4 bytes op idx + 4 bytes dep idx + 32 bytes txid
    if data.len() != 44 {
        return None;
    }

    // Check the magic bytes.
    let mut magic_bytes = [0u8; 4];
    magic_bytes.copy_from_slice(&data[..4]);
    if magic_bytes != *magic {
        return None;
    }

    // Then parse out each of the indexes we're referring to.
    let mut idx_bytes = [0u8; 4];

    idx_bytes.copy_from_slice(&data[4..8]);
    let opidx: u32 = u32::from_be_bytes(idx_bytes);

    idx_bytes.copy_from_slice(&data[8..12]);
    let depidx: u32 = u32::from_be_bytes(idx_bytes);

    let deposit_txid_bytes = data[12..].try_into().unwrap();

    Some((opidx, depidx, deposit_txid_bytes))
}

/// Makes sure the scriptbuf is an OP_RETURN, and then returns the PUSHDATA
/// bytes.
fn extract_op_return_data(script: &ScriptBuf) -> Option<&[u8]> {
    use bitcoin::{
        opcodes::{Class, ClassifyContext},
        script::Instruction,
    };

    let mut isns = script.instructions();

    let first_isn = isns.next()?.ok()?;
    let isn_class = first_isn.opcode()?.classify(ClassifyContext::Legacy);

    if isn_class != Class::ReturnOp {
        return None;
    }

    let second_isn = isns.next()?.ok()?;
    match second_isn {
        Instruction::PushBytes(push_bytes) => Some(push_bytes.as_bytes()),
        Instruction::Op(_) => todo!(),
    }
}

#[cfg(test)]
mod test {
    use bitcoin::{absolute::LockTime, transaction::Version, Amount, Transaction, TxOut};
    use strata_primitives::params::Params;
    use strata_test_utils::{bitcoin::generate_withdrawal_fulfillment_data, l2::gen_params};

    use super::*;
    use crate::{
        filter::types::OPERATOR_FEE,
        utils::test_utils::{
            create_opreturn_metadata_for_withdrawal_fulfillment,
            get_filter_config_from_deposit_entries,
        },
    };

    // First 4 bytes of "strata"?  I am not sure what this is supposed to be.
    const MAGIC: [u8; 4] = [b's', b't', b'r', b'a'];

    const DEPOSIT_AMT: Amount = Amount::from_int_btc(10);

    fn deposit_amt() -> BitcoinAmount {
        DEPOSIT_AMT.into()
    }

    fn withdraw_amt_after_fees() -> Amount {
        DEPOSIT_AMT - OPERATOR_FEE
    }

    fn create_opreturn_metadata(
        magic: [u8; 4],
        operator_idx: u32,
        deposit_idx: u32,
        deposit_txid: &[u8; 32],
    ) -> ScriptBuf {
        strata_test_utils::create_opreturn_metadata(magic, operator_idx, deposit_idx, deposit_txid)
    }

    fn create_outputref(txid_bytes: &[u8; 32], vout: u32) -> OutputRef {
        OutPoint::new(consensus::deserialize(txid_bytes).unwrap(), vout).into()
    }

    fn generate_data() -> (Vec<Descriptor>, Vec<[u8; 32]>, TxFilterConfig) {
        let params: Params = gen_params();
        let mut gen = ArbitraryGenerator::new();
        let mut addresses = Vec::new();
        let mut txids = Vec::<[u8; 32]>::new();
        for _ in 0..10 {
            addresses.push(Descriptor::new_p2wpkh(&gen.generate()));
            txids.push(gen.generate());
        }

        let mut filterconfig = TxFilterConfig::derive_from(params.rollup()).unwrap();

        let create_dispatched_deposit_entry =
            |operator_idx: u32,
             deposit_idx: u32,
             addr: Descriptor,
             deadline: u64,
             deposit_txid: &[u8; 32],
             withdrawal_request_txid: Option<Buf32>| {
                DepositEntry::new(
                    deposit_idx,
                    create_outputref(deposit_txid, 0),
                    vec![0, 1, 2],
                    deposit_amt(),
                    withdrawal_request_txid,
                )
                .with_state(DepositState::Dispatched(DispatchedState::new(
                    DispatchCommand::new(vec![WithdrawOutput::new(
                        addr,
                        Amount::from_btc(10.0).unwrap().into(),
                    )]),
                    operator_idx,
                    deadline,
                )))
            };

        let deposits = vec![
            // deposits with withdrawal assignments
            create_dispatched_deposit_entry(
                1,
                2,
                addresses[0].clone(),
                100,
                &txids[0],
                gen.generate(),
            ),
            create_dispatched_deposit_entry(
                2,
                3,
                addresses[1].clone(),
                100,
                &txids[1],
                gen.generate(),
            ),
            create_dispatched_deposit_entry(
                0,
                4,
                addresses[2].clone(),
                100,
                &txids[2],
                gen.generate(),
            ),
            // deposits without withdrawal assignments
            DepositEntry::new(
                5,
                create_outputref(&txids[3], 0),
                vec![0, 1, 2],
                deposit_amt(),
                None,
            )
            .with_state(DepositState::Accepted),
            DepositEntry::new(
                6,
                create_outputref(&txids[4], 0),
                vec![0, 1, 2],
                deposit_amt(),
                None,
            )
            .with_state(DepositState::Accepted),
        ];

        // Watch all withdrawals that have been ordered.
        let exp_fulfillments = deposits
            .iter()
            .flat_map(conv_deposit_to_fulfillment)
            .collect::<Vec<_>>();

        filterconfig.expected_withdrawal_fulfillments =
            FlatTable::try_from_unsorted(exp_fulfillments).expect("types: malformed deposits");

        (addresses, txids, filterconfig)
    }

    #[test]
    fn test_parse_withdrawal_fulfillment_transactions_ok() {
        let params: Params = gen_params();
        let (addresses, txids, deposit_entries) =
            generate_withdrawal_fulfillment_data(deposit_amt());
        let filterconfig = get_filter_config_from_deposit_entries(params, &deposit_entries);

        let txn = Transaction {
            version: Version(1),
            lock_time: LockTime::from_height(0).unwrap(),
            input: vec![], // dont care
            output: vec![
                // front payment
                TxOut {
                    script_pubkey: addresses[0].to_script(),
                    value: withdraw_amt_after_fees(),
                },
                // metadata with operator index
                TxOut {
                    script_pubkey: create_opreturn_metadata(MAGIC, 1, 2, &txids[0]),
                    value: Amount::from_sat(0),
                },
                // change
                TxOut {
                    script_pubkey: addresses[4].to_script(),
                    value: Amount::from_btc(0.12345).unwrap(),
                },
            ],
        };

        let withdrawal_fulfillment_info =
            try_parse_tx_as_withdrawal_fulfillment(&txn, &filterconfig);
        assert!(withdrawal_fulfillment_info.is_some());

        assert_eq!(
            withdrawal_fulfillment_info.unwrap(),
            WithdrawalFulfillmentInfo {
                deposit_idx: 2,
                operator_idx: 1,
                amt: withdraw_amt_after_fees().into(),
                txid: txn.compute_txid().into()
            }
        );
    }

    #[test]
    fn test_parse_withdrawal_fulfillment_transactions_fail_wrong_order() {
        // TESTCASE: valid withdrawal, but different order of txout
        let params: Params = gen_params();
        let (addresses, txids, deposit_entries) =
            generate_withdrawal_fulfillment_data(deposit_amt());
        let filterconfig = get_filter_config_from_deposit_entries(params, &deposit_entries);

        let txn = Transaction {
            version: Version(1),
            lock_time: LockTime::from_height(0).unwrap(),
            input: vec![], // dont care
            output: vec![
                // change
                TxOut {
                    script_pubkey: addresses[4].to_script(),
                    value: Amount::from_btc(0.12345).unwrap(),
                },
                // metadata with operator index
                TxOut {
                    script_pubkey: create_opreturn_metadata(MAGIC, 1, 2, &txids[0]),
                    value: Amount::from_sat(0),
                },
                // front payment
                TxOut {
                    script_pubkey: addresses[0].to_script(),
                    value: withdraw_amt_after_fees(),
                },
            ],
        };

        let withdrawal_fulfillment_info =
            try_parse_tx_as_withdrawal_fulfillment(&txn, &filterconfig);
        assert!(withdrawal_fulfillment_info.is_none());
    }

    #[test]
    fn test_parse_withdrawal_fulfillment_transactions_fail_wrong_operator() {
        // TESTCASE: correct amount but wrong operator idx for deposit
        let params: Params = gen_params();
        let (addresses, txids, deposit_entries) =
            generate_withdrawal_fulfillment_data(deposit_amt());
        let filterconfig = get_filter_config_from_deposit_entries(params, &deposit_entries);

        let txn = Transaction {
            version: Version(1),
            lock_time: LockTime::from_height(0).unwrap(),
            input: vec![], // dont care
            output: vec![
                // front payment
                TxOut {
                    script_pubkey: addresses[0].to_script(),
                    value: withdraw_amt_after_fees(),
                },
                // metadata with operator index
                TxOut {
                    script_pubkey: create_opreturn_metadata(MAGIC, 2, 2, &txids[0]),
                    value: Amount::from_sat(0),
                },
                // change
                TxOut {
                    script_pubkey: addresses[4].to_script(),
                    value: Amount::from_btc(0.12345).unwrap(),
                },
            ],
        };

        let withdrawal_fulfillment_info =
            try_parse_tx_as_withdrawal_fulfillment(&txn, &filterconfig);
        assert!(withdrawal_fulfillment_info.is_none());
    }

    #[test]
    fn test_parse_withdrawal_fulfillment_transactions_fail_wrong_deposit_txid() {
        // TESTCASE: correct amount and operator idx for deposit, but wrong deposit txid
        let params: Params = gen_params();
        let (addresses, txids, deposit_entries) =
            generate_withdrawal_fulfillment_data(deposit_amt());
        let filterconfig = get_filter_config_from_deposit_entries(params, &deposit_entries);

        let txn = Transaction {
            version: Version(1),
            lock_time: LockTime::from_height(0).unwrap(),
            input: vec![], // dont care
            output: vec![
                // front payment
                TxOut {
                    script_pubkey: addresses[0].to_script(),
                    value: withdraw_amt_after_fees(),
                },
                // metadata with operator index
                TxOut {
                    script_pubkey: create_opreturn_metadata(MAGIC, 1, 2, &txids[5]),
                    value: Amount::from_sat(0),
                },
                // change
                TxOut {
                    script_pubkey: addresses[4].to_script(),
                    value: Amount::from_btc(0.12345).unwrap(),
                },
            ],
        };

        let withdrawal_fulfillment_info =
            try_parse_tx_as_withdrawal_fulfillment(&txn, &filterconfig);
        assert!(withdrawal_fulfillment_info.is_none());
    }

    #[test]
    fn test_parse_withdrawal_fulfillment_transactions_fail_missing_op_return() {
        let params: Params = gen_params();
        let (addresses, _, deposit_entries) = generate_withdrawal_fulfillment_data(deposit_amt());
        let filterconfig = get_filter_config_from_deposit_entries(params, &deposit_entries);

        let txn = Transaction {
            version: Version(1),
            lock_time: LockTime::from_height(0).unwrap(),
            input: vec![],
            output: vec![
                TxOut {
                    script_pubkey: addresses[0].to_script(),
                    value: withdraw_amt_after_fees(),
                },
                // change
                TxOut {
                    script_pubkey: addresses[5].to_script(),
                    value: Amount::from_btc(0.12345).unwrap(),
                },
            ],
        };

        let withdrawal_fulfillment_info =
            try_parse_tx_as_withdrawal_fulfillment(&txn, &filterconfig);
        assert!(withdrawal_fulfillment_info.is_none())
    }
}
