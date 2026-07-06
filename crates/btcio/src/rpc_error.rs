use std::fmt;

use anyhow::Error as AnyhowError;
use bitcoind_async_client::error::ClientError;

use crate::writer::builder::EnvelopeError;

/// Returns `true` when a Bitcoin RPC error may be resolved by retrying later.
pub(crate) fn is_retryable_client_error(err: &ClientError) -> bool {
    err.is_retriable() || is_bitcoind_warmup_error(err)
}

/// Returns `true` when bitcoind is reachable but still in RPC warmup.
pub fn is_bitcoind_warmup_error(err: &ClientError) -> bool {
    matches!(err, ClientError::Server(-28, _))
}

/// Returns `true` when an [`anyhow::Error`] wraps a retryable Bitcoin RPC error.
pub(crate) fn is_retryable_anyhow_error(err: &AnyhowError) -> bool {
    if err.chain().any(|cause| {
        cause
            .downcast_ref::<ClientError>()
            .is_some_and(is_retryable_client_error)
    }) {
        return true;
    }

    err.chain().any(|cause| {
        cause
            .downcast_ref::<reqwest::Error>()
            .is_some_and(|err| err.is_connect() || err.is_timeout())
    })
}

/// Returns `true` when an envelope error represents a retryable Bitcoin RPC outage.
pub(crate) fn is_retryable_envelope_error(err: &EnvelopeError) -> bool {
    match err {
        EnvelopeError::PrereqFetch(err) | EnvelopeError::Other(err) => {
            is_retryable_anyhow_error(err)
        }
        EnvelopeError::SignRawTransaction(err) => is_retryable_client_error(err),
        EnvelopeError::EmptyPayload
        | EnvelopeError::FeeOverflow
        | EnvelopeError::NotEnoughUtxos(_, _)
        | EnvelopeError::MissingEnvelopePubkey
        | EnvelopeError::P2trChangeAddressUnsupported
        | EnvelopeError::Taproot(_)
        | EnvelopeError::Tag(_)
        | EnvelopeError::EnvelopeBuild(_)
        | EnvelopeError::Sighash(_) => false,
    }
}

/// Formats a retryable error reason for status and logs.
pub(crate) fn retryable_reason(err: impl fmt::Display) -> String {
    err.to_string()
}

#[cfg(test)]
mod tests {
    use bitcoind_async_client::error::ClientError;

    use super::{
        is_retryable_anyhow_error, is_retryable_client_error, is_retryable_envelope_error,
        retryable_reason,
    };
    use crate::writer::builder::EnvelopeError;

    #[test]
    fn client_errors_include_bitcoind_warmup() {
        assert!(is_retryable_client_error(&ClientError::Connection(
            "connection refused".into()
        )));
        assert!(is_retryable_client_error(&ClientError::Server(
            -28,
            "Loading block index...".into()
        )));
        assert!(!is_retryable_client_error(&ClientError::Server(
            -25,
            "bad-txns-inputs-missingorspent".into()
        )));
    }

    #[test]
    fn anyhow_context_preserves_retryable_client_error_classification() {
        let err = anyhow::Error::from(ClientError::Connection("connection refused".into()))
            .context("failed to fetch envelope prerequisites");

        assert!(is_retryable_anyhow_error(&err));
    }

    #[test]
    fn anyhow_context_preserves_bitcoind_warmup_classification() {
        let err = anyhow::Error::from(ClientError::Server(-28, "Loading block index...".into()))
            .context("failed to poll Bitcoin client");

        assert!(is_retryable_anyhow_error(&err));
    }

    #[test]
    fn envelope_prereq_fetch_uses_wrapped_retry_classification() {
        let source = anyhow::Error::from(ClientError::Timeout).context("network unavailable");
        let err = EnvelopeError::PrereqFetch(source);

        assert!(is_retryable_envelope_error(&err));
    }

    #[test]
    fn envelope_signing_rpc_outages_are_retryable() {
        let err =
            EnvelopeError::SignRawTransaction(ClientError::Connection("connection refused".into()));

        assert!(is_retryable_envelope_error(&err));
    }

    #[test]
    fn envelope_signing_bitcoind_warmup_is_retryable() {
        let err = EnvelopeError::SignRawTransaction(ClientError::Server(
            -28,
            "Loading block index...".into(),
        ));

        assert!(is_retryable_envelope_error(&err));
    }

    #[test]
    fn envelope_signing_permanent_failures_are_not_retryable() {
        let err = EnvelopeError::SignRawTransaction(ClientError::Server(
            -25,
            "bad-txns-inputs-missingorspent".into(),
        ));

        assert!(!is_retryable_envelope_error(&err));
    }

    #[test]
    fn retryable_reason_formats_display_value() {
        let reason = retryable_reason(ClientError::Timeout);

        assert_eq!(reason, "Timeout");
    }
}
