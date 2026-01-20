use std::num::NonZero;

use alloy_primitives::B256;
use alpen_ee_common::{DepositInfo, EnginePayload, PayloadBuildAttributes, PayloadBuilderEngine};
use alpen_reth_evm::subject_to_address;
use strata_acct_types::Hash;
use strata_ee_acct_types::{EeAccountState, PendingInputEntry, UpdateExtraData};
use tracing::{debug, error};

/// Extracts deposits from pending inputs, limited by `max_deposits`.
///
/// Returns a vector of [`DepositInfo`] ready for payload building.
///
/// Deposits with invalid destination addresses (SubjectIds with non-zero padding
/// in the first 12 bytes) are logged as warnings and skipped.
pub(crate) fn extract_deposits(
    pending_inputs: &[PendingInputEntry],
    max_deposits: NonZero<u8>,
) -> Vec<DepositInfo> {
    pending_inputs
        .iter()
        .filter_map(|entry| match entry {
            PendingInputEntry::Deposit(data) => {
                match subject_to_address(&data.dest()) {
                    Some(address) => Some(DepositInfo::new(0, address, data.value())),
                    None => {
                        // FIXME: https://alpenlabs.atlassian.net/browse/STR-2125
                        error!(
                            subject_id = %data.dest(),
                            "Skipping deposit with invalid destination address (non-zero padding in first 12 bytes)"
                        );
                        None
                    }
                }
            }
        })
        .take(max_deposits.get() as usize)
        .collect()
}

/// Builds the block payload.
/// All EE <-> EVM conversions should be contained inside here.
pub(crate) async fn build_exec_payload<E: PayloadBuilderEngine>(
    account_state: &mut EeAccountState,
    parent_exec_blkid: Hash,
    timestamp_ms: u64,
    max_deposits_per_block: NonZero<u8>,
    payload_builder: &E,
) -> eyre::Result<(E::TEnginePayload, UpdateExtraData)> {
    let parent = B256::from_slice(parent_exec_blkid.as_slice());
    let timestamp_sec = timestamp_ms / 1_000;

    let deposits = extract_deposits(account_state.pending_inputs(), max_deposits_per_block);
    let processed_inputs = deposits.len() as u32;
    // dont handle forced inclusions currently
    let processed_fincls = 0;

    debug!(%parent, timestamp = %timestamp_sec, deposits = %processed_inputs, "starting payload build");
    let payload = payload_builder
        .build_payload(PayloadBuildAttributes::new(parent, timestamp_sec, deposits))
        .await?;

    let new_blockhash = payload.blockhash();
    debug!(?new_blockhash, "payload build complete");

    let extra_data = UpdateExtraData::new(new_blockhash, processed_inputs, processed_fincls);

    Ok((payload, extra_data))
}

#[cfg(test)]
mod tests {
    use alloy_primitives::Address;
    use strata_acct_types::{BitcoinAmount, SubjectId};
    use strata_ee_chain_types::SubjectDepositData;

    use super::*;

    fn make_deposit(dest_bytes: [u8; 32], sats: u64) -> PendingInputEntry {
        PendingInputEntry::Deposit(SubjectDepositData::new(
            SubjectId::new(dest_bytes),
            BitcoinAmount::from_sat(sats),
        ))
    }

    #[test]
    fn extract_deposits_with_valid_address() {
        // SubjectId with valid EVM address: [0x00..0x00 (12 bytes), 0xaa..0xaa (20 bytes)]
        let mut subject_bytes = [0u8; 32];
        subject_bytes[12..32].copy_from_slice(&[0xaa; 20]);

        let inputs = vec![make_deposit(subject_bytes, 1000)];
        let deposits = extract_deposits(&inputs, NonZero::new(10).unwrap());

        assert_eq!(deposits.len(), 1);
        assert_eq!(deposits[0].address(), Address::from([0xaa; 20]));
    }

    #[test]
    fn extract_deposits_skips_invalid_address() {
        // SubjectId with invalid padding: [0xff..0xff (12 bytes), 0xbb..0xbb (20 bytes)]
        let mut invalid_subject = [0u8; 32];
        invalid_subject[0..12].copy_from_slice(&[0xff; 12]);
        invalid_subject[12..32].copy_from_slice(&[0xbb; 20]);

        // Valid SubjectId: [0x00..0x00 (12 bytes), 0xaa..0xaa (20 bytes)]
        let mut valid_subject = [0u8; 32];
        valid_subject[12..32].copy_from_slice(&[0xaa; 20]);

        let inputs = vec![
            make_deposit(invalid_subject, 1000),
            make_deposit(valid_subject, 2000),
        ];
        let deposits = extract_deposits(&inputs, NonZero::new(10).unwrap());

        // Only the valid deposit should be extracted
        assert_eq!(deposits.len(), 1);
        assert_eq!(deposits[0].address(), Address::from([0xaa; 20]));
    }

    #[test]
    fn extract_deposits_limits_to_max() {
        // Create valid SubjectIds with zero-padded first 12 bytes
        let mut subject1 = [0u8; 32];
        subject1[12..32].copy_from_slice(&[0x01; 20]);
        let mut subject2 = [0u8; 32];
        subject2[12..32].copy_from_slice(&[0x02; 20]);
        let mut subject3 = [0u8; 32];
        subject3[12..32].copy_from_slice(&[0x03; 20]);
        let mut subject4 = [0u8; 32];
        subject4[12..32].copy_from_slice(&[0x04; 20]);
        let mut subject5 = [0u8; 32];
        subject5[12..32].copy_from_slice(&[0x05; 20]);

        let inputs = vec![
            make_deposit(subject1, 1000),
            make_deposit(subject2, 2000),
            make_deposit(subject3, 3000),
            make_deposit(subject4, 4000),
            make_deposit(subject5, 5000),
        ];
        let max = NonZero::new(3).unwrap();

        let deposits = extract_deposits(&inputs, max);

        assert_eq!(deposits.len(), 3);
        // Verify order is preserved (first 3)
        assert_eq!(deposits[0].amount(), BitcoinAmount::from_sat(1000));
        assert_eq!(deposits[1].amount(), BitcoinAmount::from_sat(2000));
        assert_eq!(deposits[2].amount(), BitcoinAmount::from_sat(3000));
    }
}
