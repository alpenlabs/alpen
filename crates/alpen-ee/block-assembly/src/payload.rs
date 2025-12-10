use std::num::NonZero;

use alloy_primitives::{Address, B256};
use alpen_ee_common::{DepositInfo, EnginePayload, PayloadBuildAttributes, PayloadBuilderEngine};
use strata_acct_types::{Hash, SubjectId};
use strata_ee_acct_types::{EeAccountState, PendingInputEntry, UpdateExtraData};
use tracing::debug;

/// Converts a [`SubjectId`] (32 bytes) to an EVM [`Address`] (20 bytes).
///
/// Takes the last 20 bytes of the SubjectId to form the address.
pub(crate) fn subject_id_to_address(subject_id: SubjectId) -> Address {
    let subject_bytes: [u8; 32] = subject_id.into();
    let mut address_bytes = [0u8; 20];
    address_bytes.copy_from_slice(&subject_bytes[12..32]);
    Address::from(address_bytes)
}

/// Extracts deposits from pending inputs, limited by `max_deposits`.
///
/// Returns a vector of [`DepositInfo`] ready for payload building.
pub(crate) fn extract_deposits(
    pending_inputs: &[PendingInputEntry],
    max_deposits: NonZero<u8>,
) -> Vec<DepositInfo> {
    pending_inputs
        .iter()
        .map(|entry| match entry {
            PendingInputEntry::Deposit(data) => {
                DepositInfo::new(0, subject_id_to_address(data.dest()), data.value())
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
    let parent = B256::from_slice(&parent_exec_blkid);
    let timestamp_sec = timestamp_ms / 1000;

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
    use strata_acct_types::BitcoinAmount;
    use strata_ee_chain_types::SubjectDepositData;

    use super::*;

    fn make_deposit(dest_bytes: [u8; 32], sats: u64) -> PendingInputEntry {
        PendingInputEntry::Deposit(SubjectDepositData::new(
            SubjectId::new(dest_bytes),
            BitcoinAmount::from_sat(sats),
        ))
    }

    #[test]
    fn subject_id_to_address_uses_last_20_bytes() {
        // SubjectId: [0x00..0x00 (12 bytes), 0xaa..0xaa (20 bytes)]
        let mut subject_bytes = [0u8; 32];
        subject_bytes[0..12].copy_from_slice(&[0x00; 12]);
        subject_bytes[12..32].copy_from_slice(&[0xaa; 20]);

        let subject_id = SubjectId::new(subject_bytes);
        let address = subject_id_to_address(subject_id);

        assert_eq!(address, Address::from([0xaa; 20]));
    }

    #[test]
    fn subject_id_to_address_ignores_first_12_bytes() {
        // SubjectId: [0xff..0xff (12 bytes), 0xbb..0xbb (20 bytes)]
        // The first 12 bytes should be ignored
        let mut subject_bytes = [0u8; 32];
        subject_bytes[0..12].copy_from_slice(&[0xff; 12]);
        subject_bytes[12..32].copy_from_slice(&[0xbb; 20]);

        let subject_id = SubjectId::new(subject_bytes);
        let address = subject_id_to_address(subject_id);

        assert_eq!(address, Address::from([0xbb; 20]));
    }

    #[test]
    fn extract_deposits_limits_to_max() {
        let inputs = vec![
            make_deposit([0x01; 32], 1000),
            make_deposit([0x02; 32], 2000),
            make_deposit([0x03; 32], 3000),
            make_deposit([0x04; 32], 4000),
            make_deposit([0x05; 32], 5000),
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
