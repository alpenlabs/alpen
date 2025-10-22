use std::{any::Any, fmt};

use strata_l1_txfmt::SubprotocolId;
use thiserror::Error;

use crate::{AsmLogEntry, L1TxIndex};

/// Trait implemented by auxiliary request payloads registered during preprocessing.
///
/// Payloads should carry any context needed by the outer orchestration layer to fulfil the
/// request prior to transaction processing. Implementers must be `'static + Send + Sync` so they
/// can safely cross thread boundaries and be downcast later.
pub trait AuxRequestPayload: Any + Send + Sync {
    /// Accessor for downcasting to the concrete payload type.
    fn as_any(&self) -> &dyn Any;
}

impl<T> AuxRequestPayload for T
where
    T: Any + Send + Sync,
{
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Opaque wrapper representing a single auxiliary request issued by a subprotocol for a specific
/// L1 transaction.
pub struct AuxRequest {
    tx_index: L1TxIndex,
    payload: Box<dyn AuxRequestPayload>,
}

impl AuxRequest {
    /// Creates a new auxiliary request.
    pub fn new(tx_index: L1TxIndex, payload: Box<dyn AuxRequestPayload>) -> Self {
        Self { tx_index, payload }
    }

    /// Returns the originating L1 transaction index within the block.
    pub fn tx_index(&self) -> L1TxIndex {
        self.tx_index
    }

    /// Provides access to the underlying payload as a trait object for downcasting.
    pub fn payload(&self) -> &dyn AuxRequestPayload {
        self.payload.as_ref()
    }

    /// Consumes the request, yielding the inner components.
    pub fn into_inner(self) -> (L1TxIndex, Box<dyn AuxRequestPayload>) {
        (self.tx_index, self.payload)
    }
}

impl fmt::Debug for AuxRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuxRequest")
            .field("tx_index", &self.tx_index)
            .finish_non_exhaustive()
    }
}

/// Responsible for recording auxiliary requests emitted during preprocessing.
pub trait AuxInputCollector: Any {
    /// Records that the transaction at `tx_index` requires auxiliary data described by `payload`.
    fn request_aux_input(&mut self, tx_index: L1TxIndex, payload: Box<dyn AuxRequestPayload>);

    /// Exposes the collector as a `&mut dyn Any` for downcasting.
    fn as_mut_any(&mut self) -> &mut dyn Any;
}

/// Errors that can occur while resolving auxiliary data.
#[derive(Debug, Error)]
pub enum AuxResolveError {
    /// There are no aux responses registered for the requested subprotocol.
    #[error("no auxiliary responses registered for subprotocol {subprotocol}")]
    MissingSubprotocol {
        /// The subprotocol whose aux data was requested.
        subprotocol: SubprotocolId,
    },
    /// The requested transaction has no aux responses.
    #[error("subprotocol {subprotocol} has no auxiliary data for L1 transaction index {tx_index}")]
    MissingTx {
        /// Subprotocol identifier.
        subprotocol: SubprotocolId,
        /// L1 transaction index within the block.
        tx_index: L1TxIndex,
    },
    /// The available aux data does not match the expected variant.
    #[error(
        "unexpected auxiliary response type {found} for subprotocol {subprotocol}, tx index {tx_index} (expected {expected})"
    )]
    TypeMismatch {
        /// Subprotocol identifier.
        subprotocol: SubprotocolId,
        /// L1 transaction index within the block.
        tx_index: L1TxIndex,
        /// Expected variant name.
        expected: &'static str,
        /// Found variant name.
        found: &'static str,
    },
    /// Verification of the supplied MMR proof failed.
    #[error("log MMR verification failed: {0}")]
    LogProof(String),
}

/// Result alias for aux resolution operations.
pub type AuxResolveResult<T> = Result<T, AuxResolveError>;

/// Provides access to pre-fetched auxiliary responses during transaction execution.
pub trait AuxResolver {
    /// Returns all verified historical ASM logs associated with a given L1 transaction.
    fn historical_logs(&self, tx_index: L1TxIndex) -> AuxResolveResult<Vec<AsmLogEntry>>;

    /// Returns the deposit request transaction referenced by a bridge deposit, if present.
    // TODO: consider changing return type based on bridge subprotocol needs
    fn deposit_request_tx(&self, tx_index: L1TxIndex) -> AuxResolveResult<Option<Vec<u8>>>;
}
