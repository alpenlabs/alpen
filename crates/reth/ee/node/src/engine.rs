use alloy_rpc_types::engine::ForkchoiceState;

#[derive(Debug, Clone)]
pub enum EngineError {
    // TODO
    Other(String),
}

type EngineResult<T> = Result<T, EngineError>;

pub trait ConsensusEngine<TEnginePayload> {
    fn update_consensus_state(&self, update: ForkchoiceState) -> EngineResult<()>;
}
