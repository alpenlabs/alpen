use alloy_rpc_types_engine::ForkchoiceState;
use async_trait::async_trait;

use crate::{EnginePayload, ExecutionEngineError};

/// Interface for interacting with an execution engine that processes payloads
/// and tracks consensus state. Typically wraps an Engine API-compliant client.
#[async_trait]
pub trait ExecutionEngine {
    type TEnginePayload: EnginePayload;

    /// Submits an execution payload to the engine for processing.
    async fn submit_payload(
        &self,
        payload: Self::TEnginePayload,
    ) -> Result<(), ExecutionEngineError>;

    /// Updates the engine's fork choice state (head, safe, and finalized blocks).
    async fn update_consensus_state(
        &self,
        state: ForkchoiceState,
    ) -> Result<(), ExecutionEngineError>;
}
