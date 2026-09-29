use strata_acct_types::{AccountId, BitcoinAmount};
use strata_identifiers::BRIDGE_GATEWAY_ACCT_ID;
use strata_ol_stf_v1::test_utils::{
    OLStfFixture, SnarkUpdateBuilder, make_gam_tx, make_p2wpkh_bosd_descriptor, make_proof,
    make_state_root, make_withdrawal_payload,
};

use super::*;

fn fixture_and_builder() -> (OLStfFixture, AccountId, SnarkUpdateBuilder) {
    let account = AccountId::from([0x41; 32]);
    let fixture = OLStfFixture::builder()
        .with_genesis_snark_account(account, |builder| {
            builder.with_balance(BitcoinAmount::try_from(100_000 * 100_000_000u64).unwrap())
        })
        .execute_genesis();
    let builder =
        SnarkUpdateBuilder::from_snark_state(fixture.expect_snark_account(account).clone());
    (fixture, account, builder)
}

fn long_descriptor() -> Vec<u8> {
    let mut descriptor = vec![0x42; 81];
    descriptor[0] = 0; // OP_RETURN BOSD with 80 data bytes.
    descriptor
}

fn withdrawal_update(count: usize, extra_len: usize) -> OLTransactionV1 {
    let (_, account, mut builder) = fixture_and_builder();
    let payload = make_withdrawal_payload(long_descriptor());
    for _ in 0..count {
        builder = builder.with_output_message(
            BRIDGE_GATEWAY_ACCT_ID,
            BridgeParams::default().denomination(),
            payload.clone(),
        );
    }
    builder
        .try_with_extra_data(vec![0x77; extra_len])
        .unwrap()
        .build(account, make_state_root(2), make_proof(1))
}

fn predict_log_usage(tx: &OLTransactionV1, params: &BridgeParams) -> LogUsage {
    let mut usage = LogUsage::default();
    predict_tx_log_payloads(OLSpecId::V1, tx, params, |payload| {
        usage.add_payload(payload);
    })
    .unwrap();
    usage
}

#[test]
fn test_unimplemented_spec_preserves_prediction_error() {
    let (_, account, builder) = fixture_and_builder();
    let tx = builder.build(account, make_state_root(2), make_proof(1));
    let error = check_tx_log_budget(OLSpecId::V0, &tx, &BridgeParams::default())
        .expect_err("V0 rules are not implemented");

    assert!(matches!(
        error,
        TxLogBudgetError::Prediction(ExecError::UnimplementedSpec(OLSpecId::V0))
    ));
}

#[test]
fn test_payload_budget_includes_update_log_and_extra_data() {
    let params = BridgeParams::default();
    for (extra_len, expected_bytes) in [(0, 16_350), (33, 16_383)] {
        let tx = withdrawal_update(172, extra_len);
        check_tx_log_budget(OLSpecId::V1, &tx, &params).unwrap();
        let usage = predict_log_usage(&tx, &params);
        assert_eq!(usage.count(), 173);
        assert_eq!(usage.payload_bytes(), expected_bytes);
    }
    assert!(matches!(
        check_tx_log_budget(OLSpecId::V1, &withdrawal_update(172, 34), &params),
        Err(TxLogBudgetError::LogPayloadBytes {
            actual: 16_384,
            limit: 16_383
        })
    ));
    assert!(matches!(
        check_tx_log_budget(OLSpecId::V1, &withdrawal_update(173, 0), &params),
        Err(TxLogBudgetError::LogPayloadBytes {
            actual: 16_445,
            limit: 16_383
        })
    ));
}

#[test]
fn test_log_measurement_matches_stf_with_mixed_messages() {
    let (mut fixture, account, builder) = fixture_and_builder();
    let params = BridgeParams::default();
    let amount = params.denomination();
    let valid = make_withdrawal_payload(long_descriptor());
    let tx = builder
        .try_with_extra_data(vec![0x77; 200])
        .unwrap()
        .with_output_message(BRIDGE_GATEWAY_ACCT_ID, amount, valid.clone())
        .with_output_message(
            BRIDGE_GATEWAY_ACCT_ID,
            amount,
            make_withdrawal_payload(make_p2wpkh_bosd_descriptor(0x15)),
        )
        .with_output_message(BRIDGE_GATEWAY_ACCT_ID, amount + 1, valid.clone())
        .with_output_message(BRIDGE_GATEWAY_ACCT_ID, 0, valid.clone())
        .with_output_message(
            BRIDGE_GATEWAY_ACCT_ID,
            amount,
            make_withdrawal_payload(vec![3, 1, 2]),
        )
        .with_output_message(
            BRIDGE_GATEWAY_ACCT_ID,
            amount,
            make_withdrawal_payload(vec![0; 82]),
        )
        .with_output_message(BRIDGE_GATEWAY_ACCT_ID, amount, vec![])
        .with_output_message(BRIDGE_GATEWAY_ACCT_ID, amount, vec![0x7f])
        .with_output_message(account, amount, valid)
        .with_transfer(BRIDGE_GATEWAY_ACCT_ID, amount)
        .build(account, make_state_root(2), make_proof(1));

    let usage = predict_log_usage(&tx, &params);
    let output = fixture.child_block().with_tx(tx).execute_with_outputs();
    assert_eq!(output.log_count(), 3);
    assert_eq!(usage.count(), output.log_count());
    assert_eq!(
        usage.payload_bytes(),
        output
            .logs()
            .iter()
            .map(|log| log.payload().len())
            .sum::<usize>()
    );
}

#[test]
fn test_generic_message_prediction_matches_execution() {
    let (mut fixture, account, _) = fixture_and_builder();
    let tx = make_gam_tx(account);
    let params = BridgeParams::default();
    let usage = predict_log_usage(&tx, &params);
    check_tx_log_budget(OLSpecId::V1, &tx, &params).unwrap();

    let output = fixture.child_block().with_tx(tx).execute_with_outputs();
    assert_eq!(usage, LogUsage::default());
    assert_eq!(output.log_count(), 0);
}

#[test]
fn test_maximal_update_exceeds_log_count_budget() {
    // Exercise #2274's full message capacity, including the update's own log.
    let error = check_tx_log_budget(
        OLSpecId::V1,
        &withdrawal_update(65_536, 0),
        &BridgeParams::default(),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        TxLogBudgetError::LogCount {
            actual: 65_537,
            limit: 16_383
        }
    ));
}

#[test]
fn test_log_count_limit_is_inclusive_for_the_admission_error() {
    // Isolate the count dimension; real withdrawal payloads hit the byte limit sooner.
    let limit = (MAX_LOGS_PER_BLOCK as usize).min(MAX_OL_LOGS_PER_CHECKPOINT as usize - 1);
    let mut usage = LogUsage::default();
    for _ in 0..limit {
        usage.add_payload(&[]);
    }
    assert!(check_limits(&usage).is_ok());
    usage.add_payload(&[]);
    let error = check_limits(&usage).unwrap_err();
    assert!(
        matches!(error, TxLogBudgetError::LogCount { actual, limit: reported_limit }
        if actual == limit + 1 && reported_limit == limit)
    );
}
