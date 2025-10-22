use std::{any::Any, fmt};

use bitcoin::BlockHash;
use strata_l1_txfmt::SubprotocolId;
use thiserror::Error;

use crate::{AsmLogEntry, L1TxIndex};

/// Trait implemented by auxiliary request payloads requested during preprocessing.
///
/// Payloads should carry any context needed by the outer asm worker to fulfil the
/// request prior to transaction processing.
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

/// wrapper representing a single auxiliary request issued by a subprotocol for a specific
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

/// Extension trait adding ergonomic helpers for [`AuxInputCollector`] implementers.
pub trait AuxInputCollectorExt: AuxInputCollector {
    /// Convenience helper that accepts concrete payload types without manual boxing.
    fn request_aux<T>(&mut self, tx_index: L1TxIndex, payload: T)
    where
        T: AuxRequestPayload,
    {
        self.request_aux_input(tx_index, Box::new(payload));
    }
}

impl<T: AuxInputCollector + ?Sized> AuxInputCollectorExt for T {}

/// Errors that can occur while resolving auxiliary data.
#[derive(Debug, Error)]
pub enum AuxResolveError {
    /// The available aux data does not match the expected variant.
    #[error(
        "unexpected auxiliary response for subprotocol {subprotocol}, tx index {tx_index} (expected {expected:?}, provided {actual:?})"
    )]
    UnexpectedResponseVariant {
        /// Subprotocol identifier.
        subprotocol: SubprotocolId,
        /// L1 transaction index within the block.
        tx_index: L1TxIndex,
        /// Expected variant.
        expected: AuxResponseKind,
        /// Response that was provided.
        actual: AuxResponseKind,
    },
    /// Verification of the supplied MMR proof failed.
    #[error(
        "log MMR verification failed for subprotocol {subprotocol}, tx index {tx_index}, block {block_hash}"
    )]
    InvalidLogProof {
        /// Subprotocol identifier.
        subprotocol: SubprotocolId,
        /// L1 transaction index within the block.
        tx_index: L1TxIndex,
        /// Hash of the L1 block whose logs were being proven.
        block_hash: BlockHash,
    },
}

/// Result alias for aux resolution operations.
pub type AuxResolveResult<T> = Result<T, AuxResolveError>;

/// Enumerates the different auxiliary response variants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuxResponseKind {
    HistoricalLogs,
    HistoricalLogsRange,
    DepositRequestTx,
}

/// Provides access to pre-fetched auxiliary responses during transaction execution.
pub trait AuxResolver {
    /// Returns all verified historical ASM logs associated with a given L1 transaction.
    fn historical_logs(&self, tx_index: L1TxIndex) -> AuxResolveResult<Vec<AsmLogEntry>>;

    /// Returns the deposit request transaction referenced by a bridge deposit, if present.
    // TODO: consider changing return type based on bridge subprotocol needs
    fn deposit_request_tx(&self, tx_index: L1TxIndex) -> AuxResolveResult<Option<Vec<u8>>>;
}
