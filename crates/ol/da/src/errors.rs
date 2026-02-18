use bitcoin::consensus::encode;
use strata_acct_types::AccountSerial;
use strata_asm_txs_checkpoint::CheckpointTxError;
use strata_btc_types::BitcoinTxid;
use strata_codec::CodecError;
use strata_da_framework::DaError as FrameworkDaError;
use strata_l1_txfmt::TxFmtError;
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

pub type DaExtractorResult<T> = Result<T, DaExtractorError>;

#[derive(Debug, Error)]
pub enum DaExtractorError {
    #[error("failed to decode raw bitcoin transaction: {0}")]
    BitcoinTxDecodeError(#[from] encode::Error),

    #[error("failed to fetch checkpoint transaction: {0:?}")]
    BitcoinTxNotFound(BitcoinTxid),

    #[error("checkpoint transaction failed: {0}")]
    CheckpointTxError(#[from] CheckpointTxError),

    #[error("SPS-50 tag parsing failed: {0}")]
    TagParse(#[from] TxFmtError),

    #[error("OL DA payload decode failed: {0}")]
    DaPayloadDecode(#[from] CodecError),
}
