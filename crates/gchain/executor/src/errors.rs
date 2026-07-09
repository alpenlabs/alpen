use thiserror::Error;

#[derive(Debug, Error)]
pub enum GExecError {
    // TODO add link ref somehow
    #[error("missing link")]
    MissingLink,
}
