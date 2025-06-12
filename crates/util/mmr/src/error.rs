use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum MerkleError {
    #[error("no element present in merkle tree")]
    NoElements,

    #[error("not power-of-2 size")]
    NotPowerOfTwo,

    #[error("index provided out of bounds")]
    IndexOutOfBounds,

    #[error("provided chunk size too big")]
    ChunkSizeTooBig,

    #[error("MMR has reached max capacity")]
    MaxCapacity,

    #[error("unknown error")]
    Unknown,
}
