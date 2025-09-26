use thiserror::Error;

#[derive(Debug, Error)]
pub enum EnvError {
    #[error("decoding structure")]
    Decode,

    #[error("extra coinputs provided")]
    ExtraCoinputs,

    #[error("coinput invalid for msg")]
    MalformedCoinput,

    #[error("coinput exactly did not match msg")]
    MismatchedCoinput,

    #[error("coinput is internally inconsistent")]
    InconsistentCoinput,
}

pub type EnvResult<T> = Result<T, EnvError>;
