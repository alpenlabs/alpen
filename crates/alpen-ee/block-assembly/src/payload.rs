use std::num::NonZero;

use alloy_primitives::{Address, B256};
use alpen_ee_common::{DepositInfo, EnginePayload, PayloadBuildAttributes, PayloadBuilderEngine};
use strata_acct_types::Hash;
use strata_ee_acct_types::{EeAccountState, PendingInputEntry, UpdateExtraData};
use tracing::debug;

/// Extracts deposits from pending inputs, limited by `max_deposits`.
///
/// Returns a vector of [`DepositInfo`] ready for payload building.
pub(crate) fn extract_deposits(
    pending_inputs: &[PendingInputEntry],
    max_deposits: NonZero<u8>,
) -> Vec<DepositInfo> {
    pending_inputs
        .iter()
        .filter_map(|entry| match entry {
            PendingInputEntry::Deposit(data) => Some(DepositInfo::new(
                0,
                Address::from_slice(data.dest().inner()),
                data.value(),
            )),
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
