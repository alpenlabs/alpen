use alloy_rpc_types_engine::ForkchoiceState;

use crate::traits::error::ExecutionEngineError;

/// Interface for interacting with an execution engine that processes payloads
/// and tracks consensus state. Typically wraps an Engine API-compliant client.
pub(crate) trait ExecutionEngine<TEnginePayload> {
    /// Submits an execution payload to the engine for processing.
    #[expect(dead_code, reason = "can be used for custom sync")]
    async fn submit_payload(&self, payload: TEnginePayload) -> Result<(), ExecutionEngineError>;

    /// Updates the engine's fork choice state (head, safe, and finalized blocks).
    async fn update_consenesus_state(
        &self,
        state: ForkchoiceState,
    ) -> Result<(), ExecutionEngineError>;
}
