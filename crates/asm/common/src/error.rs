use std::fmt::{Debug, Display};

use strata_primitives::l1::L1VerificationError;
use thiserror::Error;

use crate::SubprotocolId;

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

    /// L1Header do not follow consensus rules.
    #[error("L1Header do not follow consensus rules")]
    InvalidL1Header(#[source] L1VerificationError),
}
