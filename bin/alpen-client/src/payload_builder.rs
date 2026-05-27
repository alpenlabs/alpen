use std::time::{Duration, Instant};

use alloy_eips::eip4895::Withdrawal;
use alloy_primitives::B256;
use alloy_rpc_types_engine::{ForkchoiceState, PayloadAttributes};
use alpen_ee_common::{
    sats_to_gwei, ExecutionEngine, ExecutionEngineError, PayloadBuildAttributes,
    PayloadBuilderEngine,
};
use alpen_ee_engine::AlpenRethExecEngine;
use alpen_reth_evm::constants::COINBASE_ADDRESS;
use alpen_reth_node::{
    AlpenBuiltPayload, AlpenEngineTypes, AlpenPayloadAttributes, AlpenPayloadBuilderAttributes,
};
use eyre::{eyre, Context};
use reth_node_builder::{ConsensusEngineHandle, PayloadBuilderAttributes};
use reth_payload_builder::{PayloadBuilderError, PayloadBuilderHandle};
use tracing::{debug, warn};

const MISSING_PARENT_RETRY_DELAY_MS: u64 = 250;
const MISSING_PARENT_MAX_RETRIES: u16 = 120;

fn is_retryable_missing_parent(err: &PayloadBuilderError) -> bool {
    matches!(
        err,
        PayloadBuilderError::MissingParentHeader(_) | PayloadBuilderError::MissingParentBlock(_)
    )
}

async fn sleep_before_missing_parent_retry(
    parent: B256,
    timestamp: u64,
    deposit_count: usize,
    retry: u16,
    total_elapsed_ms: u128,
    err: &PayloadBuilderError,
) {
    if retry == 1 {
        warn!(
            %parent,
            timestamp,
            deposit_count,
            retry,
            max_retries = MISSING_PARENT_MAX_RETRIES,
            retry_delay_ms = MISSING_PARENT_RETRY_DELAY_MS,
            total_elapsed_ms,
            error = %err,
            "payload builder parent is not ready; retrying"
        );
    } else {
        debug!(
            %parent,
            timestamp,
            deposit_count,
            retry,
            max_retries = MISSING_PARENT_MAX_RETRIES,
            retry_delay_ms = MISSING_PARENT_RETRY_DELAY_MS,
            total_elapsed_ms,
            error = %err,
            "payload builder parent is still not ready; retrying"
        );
    }

    tokio::time::sleep(Duration::from_millis(MISSING_PARENT_RETRY_DELAY_MS)).await;
}

#[derive(Debug)]
pub(crate) struct AlpenRethPayloadEngine {
    payload_builder_handle: PayloadBuilderHandle<AlpenEngineTypes>,
    exec_engine: AlpenRethExecEngine,
}

impl AlpenRethPayloadEngine {
    pub(crate) fn new(
        payload_builder_handle: PayloadBuilderHandle<AlpenEngineTypes>,
        beacon_engine_handle: ConsensusEngineHandle<AlpenEngineTypes>,
    ) -> Self {
        Self {
            payload_builder_handle,
            exec_engine: AlpenRethExecEngine::new(beacon_engine_handle),
        }
    }
}

#[async_trait::async_trait]
impl ExecutionEngine for AlpenRethPayloadEngine {
    type TEnginePayload = AlpenBuiltPayload;

    async fn submit_payload(&self, payload: AlpenBuiltPayload) -> Result<(), ExecutionEngineError> {
        self.exec_engine.submit_payload(payload).await
    }

    async fn update_consensus_state(
        &self,
        state: ForkchoiceState,
    ) -> Result<(), ExecutionEngineError> {
        self.exec_engine.update_consensus_state(state).await
    }
}

#[async_trait::async_trait]
impl PayloadBuilderEngine for AlpenRethPayloadEngine {
    async fn build_payload(
        &self,
        build_attrs: PayloadBuildAttributes,
    ) -> eyre::Result<AlpenBuiltPayload> {
        let parent = build_attrs.parent();
        let timestamp = build_attrs.timestamp();
        let deposit_count = build_attrs.deposits().len();
        let withdrawals = build_attrs
            .deposits()
            .iter()
            .map(|deposit| {
                Ok::<Withdrawal, eyre::Error>(Withdrawal {
                    // Index fields are set to 0 because Alpen uses the Withdrawal type
                    // to transfer deposits into the EVM state, not for validator withdrawals.
                    // These indices are unused in our execution context.
                    index: 0,
                    validator_index: 0,
                    address: deposit.address(),
                    amount: sats_to_gwei(deposit.amount().to_sat())
                        .ok_or(eyre!("invalid deposit amount"))?,
                })
            })
            .collect::<Result<Vec<Withdrawal>, _>>()?;
        let payload_attrs = AlpenPayloadAttributes::new_from_eth(PayloadAttributes {
            timestamp: build_attrs.timestamp(),
            // IMPORTANT: post cancun payload build will fail without
            // parent_beacon_block_root
            parent_beacon_block_root: Some(B256::ZERO),
            prev_randao: B256::ZERO,
            suggested_fee_recipient: COINBASE_ADDRESS,
            withdrawals: Some(withdrawals),
        });

        let payload_builder_attrs =
            AlpenPayloadBuilderAttributes::try_new(parent, payload_attrs, 0)?;

        let build_started = Instant::now();
        debug!(
            %parent,
            timestamp,
            deposit_count,
            "requesting payload builder job"
        );
        let mut missing_parent_retries = 0;
        let payload = loop {
            let payload_id = match self
                .payload_builder_handle
                .send_new_payload(payload_builder_attrs.clone())
                .await
            {
                Ok(Ok(payload_id)) => {
                    debug!(
                        %parent,
                        timestamp,
                        deposit_count,
                        ?payload_id,
                        elapsed_ms = build_started.elapsed().as_millis(),
                        "payload builder accepted job"
                    );
                    payload_id
                }
                Ok(Err(err))
                    if is_retryable_missing_parent(&err)
                        && missing_parent_retries < MISSING_PARENT_MAX_RETRIES =>
                {
                    missing_parent_retries += 1;
                    sleep_before_missing_parent_retry(
                        parent,
                        timestamp,
                        deposit_count,
                        missing_parent_retries,
                        build_started.elapsed().as_millis(),
                        &err,
                    )
                    .await;
                    continue;
                }
                Ok(Err(err)) => {
                    warn!(
                        %parent,
                        timestamp,
                        deposit_count,
                        elapsed_ms = build_started.elapsed().as_millis(),
                        error = %err,
                        "payload builder rejected job"
                    );
                    return Err(err).context("failed to build payload");
                }
                Err(err) => {
                    warn!(
                        %parent,
                        timestamp,
                        deposit_count,
                        elapsed_ms = build_started.elapsed().as_millis(),
                        error = %err,
                        "failed to communicate with payload builder"
                    );
                    return Err(err).context("failed to communicate with payload builder");
                }
            };

            let resolve_started = Instant::now();
            match self
                .payload_builder_handle
                .resolve_kind(payload_id, reth_node_builder::PayloadKind::WaitForPending)
                .await
            {
                Some(Ok(payload)) => {
                    debug!(
                        %parent,
                        timestamp,
                        deposit_count,
                        ?payload_id,
                        resolve_elapsed_ms = resolve_started.elapsed().as_millis(),
                        total_elapsed_ms = build_started.elapsed().as_millis(),
                        "payload builder resolved payload"
                    );
                    break payload;
                }
                Some(Err(err))
                    if is_retryable_missing_parent(&err)
                        && missing_parent_retries < MISSING_PARENT_MAX_RETRIES =>
                {
                    missing_parent_retries += 1;
                    sleep_before_missing_parent_retry(
                        parent,
                        timestamp,
                        deposit_count,
                        missing_parent_retries,
                        build_started.elapsed().as_millis(),
                        &err,
                    )
                    .await;
                }
                Some(Err(err)) => {
                    warn!(
                        %parent,
                        timestamp,
                        deposit_count,
                        ?payload_id,
                        resolve_elapsed_ms = resolve_started.elapsed().as_millis(),
                        total_elapsed_ms = build_started.elapsed().as_millis(),
                        error = %err,
                        "payload builder failed while resolving payload"
                    );
                    return Err(err).context("failed build payload");
                }
                None => {
                    warn!(
                        %parent,
                        timestamp,
                        deposit_count,
                        ?payload_id,
                        resolve_elapsed_ms = resolve_started.elapsed().as_millis(),
                        total_elapsed_ms = build_started.elapsed().as_millis(),
                        "payload builder returned no payload"
                    );
                    return Err(eyre::eyre!("build payload missing"));
                }
            }
        };

        Ok(payload)
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::B256;

    use super::*;

    #[test]
    fn missing_parent_header_errors_are_retryable() {
        let err = PayloadBuilderError::MissingParentHeader(B256::repeat_byte(0x11));

        assert!(is_retryable_missing_parent(&err));
    }

    #[test]
    fn missing_parent_block_errors_are_retryable() {
        let err = PayloadBuilderError::MissingParentBlock(B256::repeat_byte(0x22));

        assert!(is_retryable_missing_parent(&err));
    }

    #[test]
    fn missing_payload_errors_are_not_retryable_as_missing_parent() {
        let err = PayloadBuilderError::MissingPayload;

        assert!(!is_retryable_missing_parent(&err));
    }
}
