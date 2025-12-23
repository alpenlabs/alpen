use std::fmt::Debug;

use strata_codec::CodecError;
use strata_l1_txfmt::{TxFmtError, TxType};
use thiserror::Error;

use crate::constants::BridgeTxType;

/// Errors that can occur when parsing bridge transaction
#[derive(Debug, Error)]
pub enum BridgeTxParseError {
    #[error("invalid bridge tx structure: {0}")]
    InvalidStructure(#[from] TxStructureError),

    #[error("tx type {0} is not directly processed (fetched as auxiliary data)")]
    NotDirectlyProcessed(TxType),

    #[error("unsupported tx type {0}")]
    UnsupportedTxType(TxType),
}

/// Specific reasons for a structural error when parsing bridge transactions.
#[derive(Debug, Error)]
pub enum TxStructureErrorKind {
    /// Missing input at the expected index.
    #[error("missing input at index {index}")]
    MissingInput { index: usize },

    /// Missing output at the expected index.
    #[error("missing output at index {index}")]
    MissingOutput { index: usize },

    /// Transaction format is invalid (failed SPS-50 parsing).
    #[error("invalid transaction format: {0}")]
    InvalidTxFormat(#[from] TxFmtError),

    /// Auxiliary data failed to decode.
    #[error("invalid auxiliary data: {0}")]
    InvalidAuxiliaryData(#[from] CodecError),

    /// Witness data failed validation.
    #[error("invalid witness: {0}")]
    InvalidWitness(#[from] WitnessError),
}

/// A generic "expected vs got" error.
#[derive(Debug, Error, Clone)]
#[error("(expected {expected:?}, got {got:?})")]
pub struct Mismatch<T>
where
    T: Debug + Clone,
{
    /// The value that was expected.
    pub expected: T,
    /// The value that was actually encountered.
    pub got: T,
}

/// Common structural parsing errors shared by bridge transactions.
#[derive(Debug, Error)]
#[error("{tx_type} tx structure error: {kind}{}", context_suffix(.context))]
pub struct TxStructureError {
    tx_type: BridgeTxType,
    #[source]
    kind: TxStructureErrorKind,
    context: Option<&'static str>,
}

fn context_suffix(context: &Option<&'static str>) -> String {
    context
        .map(|c| format!(" (context: {})", c))
        .unwrap_or_default()
}

impl TxStructureError {
    /// Create a new error for the provided transaction type and reason, with optional context.
    fn new_with_context(
        tx_type: BridgeTxType,
        kind: TxStructureErrorKind,
        context: Option<&'static str>,
    ) -> Self {
        Self {
            tx_type,
            kind,
            context,
        }
    }

    /// Transaction type associated with the error.
    pub fn tx_type(&self) -> BridgeTxType {
        self.tx_type
    }

    /// Reason describing the structural failure.
    pub fn kind(&self) -> &TxStructureErrorKind {
        &self.kind
    }

    /// Additional context for the error, if any.
    pub fn context(&self) -> Option<&'static str> {
        self.context
    }

    /// Missing input at the expected index.
    pub fn missing_input(tx_type: BridgeTxType, index: usize, context: &'static str) -> Self {
        Self::new_with_context(
            tx_type,
            TxStructureErrorKind::MissingInput { index },
            Some(context),
        )
    }

    /// Missing output at the expected index.
    pub fn missing_output(tx_type: BridgeTxType, index: usize, context: &'static str) -> Self {
        Self::new_with_context(
            tx_type,
            TxStructureErrorKind::MissingOutput { index },
            Some(context),
        )
    }

    /// Transaction format is invalid (failed SPS-50 parsing).
    pub fn invalid_tx_format(tx_type: BridgeTxType, err: TxFmtError) -> Self {
        Self::new_with_context(tx_type, TxStructureErrorKind::InvalidTxFormat(err), None)
    }

    /// Auxiliary data failed to decode.
    pub fn invalid_auxiliary_data(tx_type: BridgeTxType, err: CodecError) -> Self {
        Self::new_with_context(
            tx_type,
            TxStructureErrorKind::InvalidAuxiliaryData(err),
            None,
        )
    }

    /// Witness data failed validation.
    pub fn invalid_witness(
        tx_type: BridgeTxType,
        err: WitnessError,
        context: &'static str,
    ) -> Self {
        Self::new_with_context(
            tx_type,
            TxStructureErrorKind::InvalidWitness(err),
            Some(context),
        )
    }
}

/// Further details for invalid witness errors.
#[derive(Debug, Error)]
pub enum WitnessError {
    /// Witness length did not match expected layout.
    #[error("invalid witness length: expected {expected}, got {actual}")]
    InvalidLength { expected: usize, actual: usize },

    /// Witness script bytes failed validation.
    #[error("invalid witness script structure")]
    InvalidScriptStructure,
}

/// Errors that can occur when building transaction tag data for any bridge v1 transaction type.
///
/// This error type is used across all transaction types (commit, deposit, withdrawal, slash,
/// unstake) since they all have the same failure modes when building tag data.
#[derive(Debug, Error)]
pub enum TagDataError {
    /// Failed to encode auxiliary data.
    #[error("Failed to encode auxiliary data: {0}")]
    AuxiliaryDataEncoding(#[from] CodecError),

    /// Failed to create tag data.
    #[error("Failed to create tag data: {0}")]
    TagDataCreation(#[from] TxFmtError),
}
