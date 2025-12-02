use std::num::NonZero;

use alloy_primitives::{Address, B256};
use alpen_ee_common::{DepositInfo, EnginePayload, PayloadBuildAttributes, PayloadBuilderEngine};
use strata_acct_types::Hash;
use strata_ee_acct_types::{EeAccountState, PendingInputEntry, UpdateExtraData};
use tracing::debug;

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

    // limit number of deposits that are processed per block
    let max_deposit_capacity = account_state
        .pending_inputs()
        .len()
        .min(max_deposits_per_block.get() as usize);
    let mut deposits = Vec::with_capacity(max_deposit_capacity);
    for pending_input in account_state.pending_inputs() {
        match pending_input {
            PendingInputEntry::Deposit(subject_deposit_data) => {
                let deposit = DepositInfo::new(
                    0,
                    Address::from_slice(subject_deposit_data.dest().inner()),
                    subject_deposit_data.value(),
                );
                deposits.push(deposit);
            }
        }

        if deposits.len() == max_deposits_per_block.get() as usize {
            break;
        }
    }

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
