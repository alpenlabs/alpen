use strata_chaintsn::errors::TsnError;
use strata_db_types::DbError;
use strata_eectl::errors::EngineError;
use strata_ol_chain_types::L2BlockId;
use thiserror::Error;

/// Possible errors during block assembly and block template handling.
#[derive(Debug, Error)]
pub enum Error {
    /// Block generate was requested with timestamp earlier than acceptable.
    #[error("block timestamp too early: {0}")]
    TimestampTooEarly(u64),
    /// Request with an unknown template id.
    #[error("unknown templateid: {0}")]
    UnknownTemplateId(L2BlockId),
    /// Provided signature invalid for block template.
    #[error("invalid signature supplied for templateid: {0}")]
    InvalidSignature(L2BlockId),
    /// Could not send request to worker on channel due to rx being closed.
    #[error("failed to send request, template worker exited")]
    RequestChannelClosed,
    /// Could not receive response from worker on channel due to response tx being closed.
    #[error("failed to get response, template worker exited")]
    ResponseChannelClosed,
    /// Could not send message to FCM.
    #[error("failed to send fcm message, fcm worker exited")]
    FcmChannelClosed,
    /// Database Error.
    #[error("db: {0}")]
    DbError(#[from] DbError),

    /// Error during block assembly.
    #[error("block_assembly: {0}")]
    BlockAssemblyError(#[from] BlockAssemblyError),
}

#[derive(Debug, Error)]
pub enum BlockAssemblyError {
    #[error("missing expected chainstate for block {0:?}")]
    MissingBlockChainstate(L2BlockId),

    // This probably shouldn't happen, it would suggest the database is
    // misbehaving.
    #[error("missing expected state checkpoint at {0}")]
    MissingCheckpoint(u64),

    #[error("L1 block {0} missing from database")]
    MissingL1BlockHeight(u64),

    #[error("block assembly timed out")]
    BlockAssemblyTimedOut,

    #[error("missing L1 tip block")]
    MissingTipBlock,

    #[error("consensus: {0}")]
    ConsensusError(#[from] strata_consensus_logic::errors::Error),

    #[error("invalid state transition: {0}")]
    InvalidStateTsnImm(#[from] TsnError),

    #[error("engine: {0}")]
    Engine(#[from] EngineError),

    #[error("db: {0}")]
    Db(#[from] strata_db_types::errors::DbError),
}
