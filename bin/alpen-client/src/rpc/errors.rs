use std::fmt::Display;

use jsonrpsee::types::{
    error::{INTERNAL_ERROR_CODE, INVALID_PARAMS_CODE},
    ErrorObjectOwned,
};

/// Creates an RPC error for internal failures.
pub(crate) fn internal_error(msg: impl Into<String>) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(INTERNAL_ERROR_CODE, msg.into(), None::<()>)
}

/// Creates an RPC error for missing block hash input.
pub(crate) fn block_not_found_error() -> ErrorObjectOwned {
    ErrorObjectOwned::owned(INVALID_PARAMS_CODE, "block not found", None::<()>)
}

/// Creates an RPC error for frontier resolution failures.
pub(crate) fn frontier_unavailable_error(err: impl Display) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(INTERNAL_ERROR_CODE, err.to_string(), None::<()>)
}
