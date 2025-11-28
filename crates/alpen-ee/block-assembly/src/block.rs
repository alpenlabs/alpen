use std::num::NonZero;

use alpen_ee_common::PayloadBuilderEngine;
use alpen_reth_node::AlpenBuiltPayload;
use strata_acct_types::{AccountId, Hash};
use strata_ee_acct_runtime::{apply_final_update_changes, apply_input_messages};
use strata_ee_acct_types::EeAccountState;
use strata_ee_chain_types::ExecBlockPackage;
use strata_snark_acct_types::MessageEntry;

use crate::{package::build_block_package, payload::build_exec_payload};

/// All inputs that control the next built block.
#[derive(Debug)]
pub struct BlockAssemblyInputs {
    /// EeAccountState of last block.
    pub account_state: EeAccountState,
    /// New inbox messages to be included in this block.
    /// Can be empty.
    pub inbox_messages: Vec<MessageEntry>,
    /// Exec blkid of previous block.
    pub parent_exec_blkid: Hash,
    /// Timestamp of next block to be built in ms.
    pub timestamp_ms: u64,
    /// Max number of deposits to process per block.
    pub max_deposits_per_block: NonZero<u8>,
    /// Account id for bridge gateway on ol.
    pub bridge_gateway_account_id: AccountId,
}

/// Outputs from block assembly
#[derive(Debug)]
pub struct BlockAssemblyOutputs {
    /// Block package representing the OL inputs and outputs for this block.
    pub package: ExecBlockPackage,
    /// Block payload including full exec block body.
    pub payload: AlpenBuiltPayload,
    /// EeAccountState after applying the new block.
    pub account_state: EeAccountState,
}

/// Builds the next block using `inputs` and `payload_builder`.
pub async fn build_next_exec_block<E: PayloadBuilderEngine<AlpenBuiltPayload>>(
    inputs: BlockAssemblyInputs,
    payload_builder: &E,
) -> eyre::Result<BlockAssemblyOutputs> {
    let BlockAssemblyInputs {
        mut account_state,
        inbox_messages,
        parent_exec_blkid,
        timestamp_ms,
        max_deposits_per_block,
        bridge_gateway_account_id,
    } = inputs;

    // 1. apply new inbox messages to account state
    let parsed_inputs = apply_input_messages(&mut account_state, &inbox_messages)?;

    // 2. build exec block payload
    let (payload, update_extra_data) = build_exec_payload(
        &mut account_state,
        parent_exec_blkid,
        timestamp_ms,
        max_deposits_per_block,
        payload_builder,
    )
    .await?;

    // 3. update account state based on built payload and consumed inputs
    apply_final_update_changes(&mut account_state, &update_extra_data)?;

    // 4. build exec package
    let package = build_block_package(bridge_gateway_account_id, parsed_inputs, &payload);

    Ok(BlockAssemblyOutputs {
        package,
        payload,
        account_state,
    })
}
