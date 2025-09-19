use thiserror::Error;

#[derive(Debug, Error)]
pub enum EnvError {
    #[error("decoding structure")]
    Decode,

    #[error("extra coinputs provided")]
    ExtraCoinputs,
}

pub type EnvResult<T> = Result<T, EnvError>;
