//! DA framework errors.

use thiserror::Error;

/// Errors generated in building a diff.
#[derive(Debug, Error)]
pub enum BuilderError {
    #[error("tried to set a value out of allowed bounds")]
    OutOfBoundsValue,

    #[error("not yet implemented")]
    Unimplemented,
}
