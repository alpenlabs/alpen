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

/// Creates an RPC error for invalid parameters.
pub(crate) fn invalid_params_error(msg: impl Into<String>) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(INVALID_PARAMS_CODE, msg.into(), None::<()>)
}

/// Creates an RPC error for unavailable fee-model config.
pub(crate) fn fee_model_config_unavailable_error() -> ErrorObjectOwned {
    ErrorObjectOwned::owned(
        INTERNAL_ERROR_CODE,
        "fee model config unavailable; waiting for sequencer gossip",
        None::<()>,
    )
}
