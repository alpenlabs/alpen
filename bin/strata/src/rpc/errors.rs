use std::fmt::Display;

use jsonrpsee::types::{
    ErrorObjectOwned,
    error::{INTERNAL_ERROR_CODE, INVALID_PARAMS_CODE},
};
use strata_ol_mempool::OLMempoolError;
use tracing::*;

/// Custom error code for mempool capacity-related errors.
const MEMPOOL_CAPACITY_ERROR_CODE: i32 = -32001;

/// Creates an RPC error for database failures.
pub(crate) fn db_error(e: impl Display) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(
        INTERNAL_ERROR_CODE,
        format!("Database error: {e}"),
        None::<()>,
    )
}

/// Creates an RPC error for resource not found.
pub(crate) fn not_found_error(msg: impl Into<String>) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(INVALID_PARAMS_CODE, msg.into(), None::<()>)
}

/// Creates an RPC error for internal failures.
pub(crate) fn internal_error(msg: impl Into<String>) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(INTERNAL_ERROR_CODE, msg.into(), None::<()>)
}

/// Creates an RPC error for invalid parameters.
pub(crate) fn invalid_params_error(msg: impl Into<String>) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(INVALID_PARAMS_CODE, msg.into(), None::<()>)
}

/// Maps mempool errors to RPC errors with appropriate error codes.
pub(crate) fn map_mempool_error_to_rpc(err: OLMempoolError) -> ErrorObjectOwned {
    match &err {
        // Capacity-related errors
        OLMempoolError::MempoolFull { .. } | OLMempoolError::MempoolByteLimitExceeded { .. } => {
            ErrorObjectOwned::owned(MEMPOOL_CAPACITY_ERROR_CODE, err.to_string(), None::<()>)
        }
        // Validation errors that are user's fault
        OLMempoolError::AccountDoesNotExist { .. }
        | OLMempoolError::AccountTypeMismatch { .. }
        | OLMempoolError::TransactionTooLarge { .. }
        | OLMempoolError::TransactionExpired { .. }
        | OLMempoolError::TransactionNotMature { .. }
        | OLMempoolError::UsedSequenceNumber { .. }
        | OLMempoolError::SequenceNumberGap { .. } => invalid_params_error(err.to_string()),
        // Internal errors
        OLMempoolError::AccountStateAccess(_)
        | OLMempoolError::TransactionNotFound(_)
        | OLMempoolError::Database(_)
        | OLMempoolError::Serialization(_)
        | OLMempoolError::ServiceClosed(_)
        | OLMempoolError::StateProvider(_) => {
            error!(?err, "Internal mempool error");
            internal_error(err.to_string())
        }
    }
}
