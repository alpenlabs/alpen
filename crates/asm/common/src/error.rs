use std::fmt::{Debug, Display};

use strata_l1_txfmt::SubprotocolId;
use strata_msg_fmt::TypeId;
use strata_primitives::l1::L1VerificationError;
use thiserror::Error;

/// A generic “expected vs actual” error.
#[derive(Debug, Error)]
#[error("expected {expected}, found {actual}")]
pub struct Mismatched<T>
where
    T: Debug + Display,
{
    /// The value that was expected.
    pub expected: T,
    /// The value that was actually encountered.
    pub actual: T,
}

/// Errors that can occur while working with ASM subprotocols.
#[derive(Debug, Error)]
pub enum AsmError {
    /// Subprotocol ID of a decoded section did not match the expected subprotocol ID.
    #[error(transparent)]
    SubprotoIdMismatch(#[from] Mismatched<SubprotocolId>),

    /// Subprotocol ID of a decoded section did not match the expected subprotocol ID.
    #[error(transparent)]
    TypeIdMismatch(#[from] Mismatched<TypeId>),

    /// The requested subprotocol ID was not found.
    #[error("subproto {0:?} does not exist")]
    InvalidSubprotocol(SubprotocolId),

    /// The requested subprotocol state ID was not found.
    #[error("subproto {0:?} does not exist")]
    InvalidSubprotocolState(SubprotocolId),

    /// Failed to deserialize the state of the given subprotocol.
    #[error("failed to deserialize subprotocol {0} state: {1}")]
    Deserialization(SubprotocolId, #[source] borsh::io::Error),

    /// Failed to serialize the state of the given subprotocol.
    #[error("failed to serialize subprotocol {0} state: {1}")]
    Serialization(SubprotocolId, #[source] borsh::io::Error),

    /// Failed to deserialize data for the given TypeId.
    #[error("failed to deserialize TypeId {0:?} data: {1}")]
    TypeIdDeserialization(TypeId, #[source] borsh::io::Error),

    /// Failed to serialize data for the given TypeId.
    #[error("failed to serialize TypeId {0:?} data: {1}")]
    TypeIdSerialization(TypeId, #[source] borsh::io::Error),

    /// L1Header do not follow consensus rules.
    #[error("L1Header do not follow consensus rules")]
    InvalidL1Header(#[source] L1VerificationError),

    #[error("msg format error {0:?}")]
    MsgFmtError(#[from] strata_msg_fmt::Error),

    /// Missing genesis configuration for subprotocol
    #[error("missing genesis configuration for subprotocol {0}")]
    MissingGenesisConfig(SubprotocolId),
}

/// Wrapper result type for database operations.
pub type AsmResult<T> = Result<T, AsmError>;
