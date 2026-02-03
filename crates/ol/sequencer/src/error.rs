use strata_db_types::DbError;
use strata_primitives::OLBlockId;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Unknown block: {0}")]
    UnknownBlock(OLBlockId),

    #[error("Unknown template: {0}")]
    UnknownTemplate(OLBlockId),

    #[error("Invalid signature for template: {0}")]
    InvalidSignature(OLBlockId),

    #[error("Template expired: {0}")]
    TemplateExpired(OLBlockId),

    #[error("Missing OL block: {0}")]
    MissingOLBlock(OLBlockId),

    #[error("Database error: {0}")]
    Database(#[from] DbError),

    #[error("Consensus channel closed")]
    ConsensusChannelClosed,

    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),
}
