use strata_acct_types::{AccountId, BitcoinAmount, MAX_MESSAGES};
use strata_ol_stf_v1::test_utils::{
    OLStfFixture, SnarkUpdateBuilder, assert_verification_succeeds, make_p2wpkh_bosd_descriptor,
    make_proof, make_state_root, make_withdrawal_payload,
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

#[test]
fn test_payload_budget_includes_update_log_and_extra_data() {
    let params = BridgeParams::default();
    let usage = check_tx_log_budget(&withdrawal_update(172, 0), &params).unwrap();
    assert_eq!(usage.count(), 173);
    assert_eq!(usage.payload_bytes(), 16_350);

    let tx = withdrawal_update(172, 34);
    let usage = check_tx_log_budget(&tx, &params).unwrap();
    assert_eq!(usage.payload_bytes(), 16_384);
    for terminal in [false, true] {
        let (mut fixture, _, _) = fixture_and_builder();
        let parent = fixture.last_completed_block().header().clone();
        let mut verify_state = fixture.state().clone();
        let block = fixture.child_block().with_tx(tx.clone());
        let block = if terminal { block.terminal() } else { block };
        let output = block.execute_with_outputs();
        assert_eq!(
            output
                .logs()
                .iter()
                .map(|log| log.payload().len())
                .sum::<usize>(),
            16_384
        );
        let block = output.completed_block();
        assert_verification_succeeds(
            &mut verify_state,
            block.header(),
            Some(parent),
            block.body(),
        );
    }
    assert!(matches!(
        check_tx_log_budget(&withdrawal_update(172, 35), &params),
        Err(TxLogBudgetError::LogPayloadBytes {
            actual: 16_385,
            limit: 16_384
        })
    ));
    assert!(matches!(
        check_tx_log_budget(&withdrawal_update(173, 0), &params),
        Err(TxLogBudgetError::LogPayloadBytes {
            actual: 16_445,
            limit: 16_384
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

    let usage = check_tx_log_budget(&tx, &params).unwrap();
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
fn test_maximal_update_exceeds_log_payload_budget() {
    let error =
        check_tx_log_budget(&withdrawal_update(255, 0), &BridgeParams::default()).unwrap_err();
    assert!(matches!(
        error,
        TxLogBudgetError::LogPayloadBytes {
            actual: 24_235,
            limit: 16_384
        }
    ));
}

#[test]
fn test_maximum_message_count_fits_block_log_capacity() {
    let (mut fixture, account, mut builder) = fixture_and_builder();
    let params = BridgeParams::default();
    let payload = make_withdrawal_payload(make_p2wpkh_bosd_descriptor(0x15));
    for _ in 0..MAX_MESSAGES {
        builder = builder.with_output_message(
            BRIDGE_GATEWAY_ACCT_ID,
            params.denomination(),
            payload.clone(),
        );
    }
    let tx = builder.build(account, make_state_root(2), make_proof(1));
    let usage = check_tx_log_budget(&tx, &params).unwrap();
    let output = fixture.child_block().with_tx(tx).execute_with_outputs();
    assert_eq!(usage.count(), MAX_MESSAGES as usize + 1);
    assert_eq!(output.log_count(), usage.count());
}
