use alloy_rpc_types_engine::ForkchoiceState;
use alpen_ee_common::{ExecutionEngine, ExecutionEngineError};
use alpen_reth_node::{AlpenBuiltPayload, AlpenEngineTypes};
use async_trait::async_trait;
use reth_node_builder::{
    BuiltPayload, ConsensusEngineHandle, EngineApiMessageVersion, PayloadTypes,
};

#[derive(Debug, Clone)]
pub struct AlpenRethExecEngine {
    beacon_engine_handle: ConsensusEngineHandle<AlpenEngineTypes>,
}

impl AlpenRethExecEngine {
    pub fn new(beacon_engine_handle: ConsensusEngineHandle<AlpenEngineTypes>) -> Self {
        Self {
            beacon_engine_handle,
        }
    }
}

#[async_trait]
impl ExecutionEngine<AlpenBuiltPayload> for AlpenRethExecEngine {
    async fn submit_payload(&self, payload: AlpenBuiltPayload) -> Result<(), ExecutionEngineError> {
        self.beacon_engine_handle
            .new_payload(AlpenEngineTypes::block_to_payload(
                payload.block().to_owned(),
            ))
            .await
            .map(|_| ())
            .map_err(|e| ExecutionEngineError::payload_submission(e.to_string()))
    }

    async fn update_consenesus_state(
        &self,
        state: ForkchoiceState,
    ) -> Result<(), ExecutionEngineError> {
        self.beacon_engine_handle
            .fork_choice_updated(state, None, EngineApiMessageVersion::V4)
            .await
            .map(|_| ())
            .map_err(|e| ExecutionEngineError::fork_choice_update(e.to_string()))
    }
}
