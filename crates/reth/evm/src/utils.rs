use std::mem::size_of;

use alloy_consensus::TxReceipt;
use alloy_sol_types::SolEvent;
use alpen_reth_primitives::{WithdrawalIntent, WithdrawalIntentEvent};
use reth_primitives::{Receipt, TransactionSigned};
use revm_primitives::{alloy_primitives::Bloom, Address, Log, U256};
use strata_bridge_params::BridgeParams;
use strata_identifiers::{SubjectId, SubjectIdBytes, SUBJ_ID_LEN};
use strata_ol_bridge_types::OperatorSelection;
use strata_primitives::{bitcoin_bosd::Descriptor, buf::Buf32};
use thiserror::Error;

use crate::constants::BRIDGEOUT_PRECOMPILE_ADDRESS;

pub(crate) const fn u256_from(val: u128) -> U256 {
    U256::from_limbs([(val & ((1 << 64) - 1)) as u64, (val >> 64) as u64, 0, 0])
}

/// Number of wei per rollup BTC (1e18).
#[cfg(test)]
pub(crate) const WEI_PER_BTC: u128 = 1_000_000_000_000_000_000u128;

/// Number of wei per satoshi (1e10).
pub(crate) const WEI_PER_SAT: U256 = u256_from(10_000_000_000u128);

/// Converts wei to satoshis.
/// Returns a tuple of (satoshis, remainder_in_wei).
pub(crate) fn wei_to_sats(wei: U256) -> (U256, U256) {
    wei.div_rem(WEI_PER_SAT)
}

/// Error returned when a bridge-out log cannot be decoded into a valid withdrawal intent.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum WithdrawalIntentExtractionError {
    #[error("failed to decode withdrawal intent event for tx {txid:?}")]
    EventDecode { txid: Buf32 },
    #[error("withdrawal descriptor length {len} exceeds maximum {max} for tx {txid:?}")]
    InvalidDescriptorLength { txid: Buf32, len: usize, max: u32 },
    #[error("failed to decode withdrawal descriptor for tx {txid:?}")]
    DescriptorDecode { txid: Buf32 },
}

/// Extracts withdrawal intents from bridge-out events in transaction receipts.
/// Returns an error if a log emitted by the bridge-out precompile is malformed.
///
/// # Panics
///
/// Panics if the number of transactions does not match the number of receipts.
pub fn extract_withdrawal_intents(
    transactions: &[TransactionSigned],
    receipts: &[Receipt],
    bridge_params: &BridgeParams,
) -> Result<Vec<WithdrawalIntent>, WithdrawalIntentExtractionError> {
    assert_eq!(
        transactions.len(),
        receipts.len(),
        "transactions and receipts must have the same length"
    );

    let mut intents = Vec::new();

    for (tx, receipt) in transactions.iter().zip(receipts.iter()) {
        let txid = Buf32((*tx.hash()).into());
        for log in &receipt.logs {
            if let Some(intent) = extract_withdrawal_intent_from_log(txid, log, bridge_params)? {
                intents.push(intent);
            }
        }
    }

    Ok(intents)
}

fn extract_withdrawal_intent_from_log(
    txid: Buf32,
    log: &Log,
    bridge_params: &BridgeParams,
) -> Result<Option<WithdrawalIntent>, WithdrawalIntentExtractionError> {
    if log.address != BRIDGEOUT_PRECOMPILE_ADDRESS {
        return Ok(None);
    }

    let event = WithdrawalIntentEvent::decode_log(log)
        .map_err(|_| WithdrawalIntentExtractionError::EventDecode { txid })?;

    let descriptor_len = event.destination.len();
    if !bridge_params.validate_withdrawal_descriptor_len(descriptor_len) {
        return Err(WithdrawalIntentExtractionError::InvalidDescriptorLength {
            txid,
            len: descriptor_len,
            max: bridge_params.max_withdrawal_descriptor_len(),
        });
    }

    let destination = Descriptor::from_bytes(&event.destination)
        .map_err(|_| WithdrawalIntentExtractionError::DescriptorDecode { txid })?;

    Ok(Some(WithdrawalIntent {
        amt: event.amount,
        destination,
        selected_operator: OperatorSelection::from_raw(event.selectedOperator),
    }))
}

/// Accumulates logs bloom from all receipts in the execution output.
///
/// This is a general EVM function that combines blooms from all transaction receipts
/// into a single block-level bloom filter for efficient log filtering.
pub fn accumulate_logs_bloom(receipts: &[Receipt]) -> Bloom {
    let mut logs_bloom = Bloom::default();
    receipts.iter().for_each(|r| {
        logs_bloom.accrue_bloom(&r.bloom());
    });
    logs_bloom
}

const EVM_ADDR_LEN: usize = size_of::<Address>();

/// Converts a [`SubjectId`] to an EVM [`Address`].
///
/// EVM addresses occupy the last 20 bytes of the 32-byte [`SubjectId`].
/// The first 12 bytes must be zero for a valid EVM address.
///
/// Returns [`None`] if the first 12 bytes contain any non-zero values.
///
/// See also [`subject_to_address_unchecked`] for a version without validation.
pub fn subject_to_address(subject: &SubjectId) -> Option<Address> {
    let bytes = subject.inner();
    // Check that the first 12 bytes are zero (valid EVM address padding)
    if bytes[..SUBJ_ID_LEN - EVM_ADDR_LEN]
        .iter()
        .any(|&byte| byte != 0)
    {
        return None;
    }
    Some(subject_to_address_unchecked(subject))
}

/// Converts a [`SubjectId`] to an EVM [`Address`] without validation.
///
/// Extracts the last 20 bytes of the 32-byte [`SubjectId`] as an EVM address,
/// without checking if the first 12 bytes are zero.
///
/// Use this only when you are certain the [`SubjectId`] represents a valid EVM address,
/// or when you explicitly want to ignore non-zero padding bytes.
///
/// See also [`subject_to_address`] for a validating version.
pub fn subject_to_address_unchecked(subject: &SubjectId) -> Address {
    let bytes = subject.inner();
    // Extract the last 20 bytes as the address
    let mut address_bytes = [0u8; EVM_ADDR_LEN];
    address_bytes.copy_from_slice(&bytes[SUBJ_ID_LEN - EVM_ADDR_LEN..]);
    Address::from(address_bytes)
}

/// Converts an EVM [`Address`] to a [`SubjectId`].
///
/// The resulting [`SubjectId`] will have the address in the last 20 bytes,
/// with the first 12 bytes zero-padded.
pub fn address_to_subject(address: Address) -> SubjectId {
    let bytes = SubjectIdBytes::try_new(address.to_vec())
        .expect("Address is 20 bytes, which always fits in 32-byte SubjectId");
    bytes.to_subject_id()
}

#[cfg(test)]
mod tests {
    use alpen_reth_primitives::WithdrawalIntentEvent;
    use proptest::prelude::*;
    use revm_primitives::{Bytes, LogData};
    use strata_bridge_params::DEFAULT_MAX_WITHDRAWAL_DESCRIPTOR_LEN;

    use super::*;

    proptest! {
        #[test]
        fn test_address_subject_roundtrip(addr_bytes in prop::array::uniform20(any::<u8>())) {
            let address = Address::from(addr_bytes);
            let subject = address_to_subject(address);
            let recovered_address = subject_to_address(&subject).expect("should convert back to address");
            prop_assert_eq!(address, recovered_address);
        }

        #[test]
        fn test_subject_to_address_rejects_non_zero_padding(
            addr_bytes in prop::array::uniform20(any::<u8>()),
            padding_pos in 0usize..12,
            padding_val in 1u8..=255u8,
        ) {
            // Create a 32-byte SubjectId with non-zero in padding area (first 12 bytes)
            let mut subject_buf = [0u8; 32];
            // Put address in last 20 bytes
            subject_buf[12..32].copy_from_slice(&addr_bytes);
            // Put non-zero value in padding area
            subject_buf[padding_pos] = padding_val;

            let subject = SubjectId::new(subject_buf);
            let result = subject_to_address(&subject);
            prop_assert!(result.is_none(), "should reject subject with non-zero padding");
        }
    }

    #[test]
    fn test_zero_address_conversion() {
        let address = Address::ZERO;
        let subject = address_to_subject(address);
        let recovered = subject_to_address(&subject).expect("should convert");
        assert_eq!(address, recovered);
    }

    fn test_txid() -> Buf32 {
        Buf32([7u8; 32])
    }

    fn bridgeout_log(destination: Vec<u8>) -> Log {
        let event = WithdrawalIntentEvent {
            amount: 100,
            destination: Bytes::from(destination),
            selectedOperator: u32::MAX,
        };
        Log {
            address: BRIDGEOUT_PRECOMPILE_ADDRESS,
            data: LogData::from(&event),
        }
    }

    #[test]
    fn test_extract_withdrawal_intent_from_valid_bridgeout_log() {
        let mut bosd = vec![0u8; DEFAULT_MAX_WITHDRAWAL_DESCRIPTOR_LEN as usize];
        bosd[0] = 0x00;
        let log = bridgeout_log(bosd);

        let intent =
            extract_withdrawal_intent_from_log(test_txid(), &log, &BridgeParams::default())
                .unwrap()
                .expect("bridgeout log should produce intent");

        assert_eq!(intent.amt, 100);
        assert_eq!(intent.selected_operator, OperatorSelection::any());
    }

    #[test]
    fn test_extract_withdrawal_intent_ignores_other_addresses() {
        let mut log = bridgeout_log(vec![0x03; 21]);
        log.address = Address::ZERO;

        assert!(
            extract_withdrawal_intent_from_log(test_txid(), &log, &BridgeParams::default())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn test_extract_withdrawal_intent_fails_on_bad_event_log() {
        let log = Log {
            address: BRIDGEOUT_PRECOMPILE_ADDRESS,
            data: LogData::empty(),
        };

        let err = extract_withdrawal_intent_from_log(test_txid(), &log, &BridgeParams::default())
            .unwrap_err();

        assert!(matches!(
            err,
            WithdrawalIntentExtractionError::EventDecode { .. }
        ));
    }

    #[test]
    fn test_extract_withdrawal_intent_fails_on_oversized_descriptor() {
        let mut bosd = vec![0u8; DEFAULT_MAX_WITHDRAWAL_DESCRIPTOR_LEN as usize + 1];
        bosd[0] = 0x00;
        let log = bridgeout_log(bosd);

        let err = extract_withdrawal_intent_from_log(test_txid(), &log, &BridgeParams::default())
            .unwrap_err();

        assert!(matches!(
            err,
            WithdrawalIntentExtractionError::InvalidDescriptorLength { .. }
        ));
    }

    #[test]
    fn test_extract_withdrawal_intent_fails_on_malformed_descriptor() {
        let log = bridgeout_log(vec![0x03, 0x01, 0x02, 0x03]);

        let err = extract_withdrawal_intent_from_log(test_txid(), &log, &BridgeParams::default())
            .unwrap_err();

        assert!(matches!(
            err,
            WithdrawalIntentExtractionError::DescriptorDecode { .. }
        ));
    }
}
