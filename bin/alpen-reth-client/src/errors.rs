use thiserror::Error;

#[derive(Debug, Clone, Error)]
pub enum OlClientError {
    #[error("todo")]
    Other,
}
