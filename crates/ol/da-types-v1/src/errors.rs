use bitcoin::consensus::encode;
use strata_asm_proto_checkpoint_txs::CheckpointTxError;
use strata_codec::CodecError;
use strata_l1_txfmt::TxFmtError;
use thiserror::Error;

pub type DaExtractorResult<T> = Result<T, DaExtractorError>;

#[derive(Debug, Error)]
pub enum DaExtractorError {
    #[error("failed to decode raw bitcoin transaction: {0}")]
    BitcoinTxDecodeError(#[from] encode::Error),

    #[error("checkpoint transaction failed: {0}")]
    CheckpointTxError(#[from] CheckpointTxError),

    #[error("SPS-50 tag parsing failed: {0}")]
    TagParse(#[from] TxFmtError),

    #[error("OL DA payload decode failed: {0}")]
    DaPayloadDecode(#[from] CodecError),
}
