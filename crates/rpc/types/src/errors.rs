//! Defines an [`RpcServerError`] type to represent errors from any RPC methods as well as
//! converters to appropriate json-rpc error codes.

use jsonrpsee::types::ErrorObjectOwned;
use strata_state::id::L2BlockId;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RpcServerError {
    /// Unsupported RPCs for strata.  Some of these might need to be replaced
    /// with standard unsupported errors.
    #[error("unsupported RPC")]
    Unsupported,

    #[error("not yet implemented")]
    Unimplemented,

    #[error("client not started")]
    ClientNotStarted,

    #[error("missing L2 block {0:?}")]
    MissingL2Block(L2BlockId),

    #[error("missing L1 block manifest {0}")]
    MissingL1BlockManifest(u64),

    #[error("tried to call chain-related method before rollup genesis")]
    BeforeGenesis,

    #[error("unknown idx {0}")]
    UnknownIdx(u32),

    #[error("missing chainstate for index {0}")]
    MissingChainstate(u64),

    #[error("db: {0}")]
    Db(#[from] strata_db::errors::DbError),

    #[error("blocking task '{0}' failed for unknown reason")]
    BlockingAbort(String),

    #[error("incorrect parameters: {0}")]
    IncorrectParameters(String),

    #[error("fetch limit reached. max {0}, provided {1}")]
    FetchLimitReached(u64, u64),

    #[error("missing checkpoint in database for index {0}")]
    MissingCheckpointInDb(u64),

    #[error("Proof already created for checkpoint {0}")]
    ProofAlreadyCreated(u64),

    #[error("Invalid proof for checkpoint {0}: {1}")]
    InvalidProof(u64, String),

    /// Generic internal error message.  If this is used often it should be made
    /// into its own error type.
    #[error("{0}")]
    Other(String),

    /// Generic internal error message with a payload value.  If this is used
    /// often it should be made into its own error type.
    #[error("{0} (+data)")]
    OtherEx(String, serde_json::Value),
}

impl RpcServerError {
    pub fn code(&self) -> i32 {
        match self {
            Self::Unsupported => -32600,
            Self::Unimplemented => -32601,
            Self::IncorrectParameters(_) => -32602,
            Self::MissingL2Block(_) => -32603,
            Self::MissingChainstate(_) => -32604,
            Self::Db(_) => -32605,
            Self::ClientNotStarted => -32606,
            Self::BeforeGenesis => -32607,
            Self::FetchLimitReached(_, _) => -32608,
            Self::UnknownIdx(_) => -32608,
            Self::MissingL1BlockManifest(_) => -32609,
            Self::MissingCheckpointInDb(_) => -32610,
            Self::ProofAlreadyCreated(_) => -32611,
            Self::InvalidProof(_, _) => -32612,
            Self::BlockingAbort(_) => -32001,
            Self::Other(_) => -32000,
            Self::OtherEx(_, _) => -32000,
        }
    }
}

impl From<RpcServerError> for ErrorObjectOwned {
    fn from(val: RpcServerError) -> Self {
        let code = val.code();
        match val {
            RpcServerError::OtherEx(m, b) => {
                ErrorObjectOwned::owned::<_>(code, m.to_string(), Some(b))
            }
            _ => ErrorObjectOwned::owned::<serde_json::Value>(code, format!("{}", val), None),
        }
    }
}
