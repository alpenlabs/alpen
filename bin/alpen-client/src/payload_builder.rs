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
use reth_payload_builder::PayloadBuilderHandle;

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
        let withdrawals = build_attrs
            .deposits()
            .iter()
            .map(|deposit| {
                Ok::<Withdrawal, eyre::Error>(Withdrawal {
                    index: deposit.index(),
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
            AlpenPayloadBuilderAttributes::try_new(build_attrs.parent(), payload_attrs, 0)?;

        let payload_id = self
            .payload_builder_handle
            .send_new_payload(payload_builder_attrs)
            .await
            .context("failed to communicate with payload builder")?
            .context("failed to build payload")?;

        let payload = self
            .payload_builder_handle
            .resolve_kind(payload_id, reth_node_builder::PayloadKind::WaitForPending)
            .await
            .ok_or(eyre::eyre!("build payload missing"))?
            .context("failed build payload")?;

        Ok(payload)
    }
}
