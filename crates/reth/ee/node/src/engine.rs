use alloy_rpc_types::engine::ForkchoiceState;

#[derive(Debug, Clone)]
pub enum EngineError {
    // TODO
    Other(String),
}

pub type EngineResult<T> = Result<T, EngineError>;

pub trait ConsensusEngine {
    fn update_consensus_state(&self, update: ForkchoiceState) -> EngineResult<()>;
}
