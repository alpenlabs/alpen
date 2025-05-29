//! Utilities for JSON-RPC.

use jsonrpsee::types::ErrorObjectOwned;

pub fn to_jsonrpsee_error_object(err: Option<impl ToString>, message: &str) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(
        jsonrpsee::types::error::UNKNOWN_ERROR_CODE,
        message,
        err.map(|e| e.to_string()),
    )
}

pub fn to_jsonrpsee_error<T: ToString>(message: &'static str) -> impl Fn(T) -> ErrorObjectOwned {
    move |err: T| to_jsonrpsee_error_object(Some(err), message)
}
