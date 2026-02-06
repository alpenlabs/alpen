use strata_acct_types::AccountSerial;
use strata_da_framework::DaError as FrameworkDaError;
use thiserror::Error;

pub type DaResult<T> = Result<T, DaError>;

#[derive(Debug, Error)]
pub enum DaError {
    #[error("DA framework failure: {0}")]
    FrameworkError(#[from] FrameworkDaError),

    #[error("invalid state diff: {0}")]
    InvalidStateDiff(&'static str),

    #[error("invalid ledger diff: {0}")]
    InvalidLedgerDiff(&'static str),

    #[error("unknown serial {0:?}")]
    UnknownSerial(AccountSerial),

    #[error("{0}")]
    Other(&'static str),
}

pub type DaConsumerResult<T> = Result<T, DaConsumerError>;

#[derive(Debug, Error)]
pub enum DaConsumerError {
    #[error("failed to parse SPS-50 tag from checkpoint transaction: {0}")]
    TagParse(String),

    #[error(
        "unsupported checkpoint transaction tag: expected subprotocol {expected_subprotocol} tx_type {expected_tx_type}, got subprotocol {actual_subprotocol} tx_type {actual_tx_type}"
    )]
    UnsupportedCheckpointTag {
        expected_subprotocol: u8,
        actual_subprotocol: u8,
        expected_tx_type: u8,
        actual_tx_type: u8,
    },

    #[error("failed to decode raw bitcoin transaction: {0}")]
    TxDecode(String),

    #[error("checkpoint transaction missing inputs")]
    MissingInputs,

    #[error("checkpoint transaction missing taproot leaf script in first input witness")]
    MissingLeafScript,

    #[error("failed to parse envelope payload: {0}")]
    EnvelopeParse(String),

    #[error("failed to decode signed checkpoint payload: {0}")]
    SignedCheckpointDecode(String),

    #[error("failed to decode OL DA payload from checkpoint sidecar: {0}")]
    DaPayloadDecode(String),
}
