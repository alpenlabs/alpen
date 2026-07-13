//! Single source of truth for the retry decision.
//!
//! Maps the *typed* upstream errors — [`zkaleido::ZkVmError`] and (for remote
//! proving) [`zkaleido::RemoteProofFailureReason`] — to a [`FailureAction`].
//! Classification is a property of the error, decided here, rather than of the
//! call-site that produced it. Strategy code reports *what happened* (the typed
//! error); this module decides *what to do* about it.

use zkaleido::ZkVmError;

use crate::error::FailureAction;

/// Classify a [`ZkVmError`] into a retry decision.
pub(crate) fn classify_zkvm_error(err: &ZkVmError) -> FailureAction {
    match err {
        // Network / RPC hiccups and "not ready yet" are recoverable by retrying
        // against the same request.
        ZkVmError::NetworkRetryableError(_) | ZkVmError::ProofNotReady => {
            FailureAction::RetryResume
        }
        // The submission or environment is bad — resubmitting the same input
        // won't help.
        ZkVmError::ProofGenerationError(_)
        | ZkVmError::InvalidELF(_)
        | ZkVmError::InvalidInput(_)
        | ZkVmError::InvalidVerifyingKey(_) => FailureAction::Permanent,
        // A proof we received back is malformed, unverifiable, or unreadable.
        ZkVmError::InvalidProofReceipt(_)
        | ZkVmError::ProofVerificationError(_)
        | ZkVmError::OutputExtractionError { .. } => FailureAction::Permanent,
        // Guest execution fault.
        ZkVmError::ExecutionError(_) => FailureAction::Permanent,
        // Uncategorized — retry conservatively, bounded by the task-level budget.
        ZkVmError::Other(_) => FailureAction::RetryResume,
    }
}

/// Classify a remote proof-failure reason into a retry decision.
#[cfg(feature = "remote")]
pub(crate) fn classify_remote_failure(
    reason: &zkaleido::RemoteProofFailureReason,
) -> FailureAction {
    use zkaleido::RemoteProofFailureReason as R;
    match reason {
        // The guest can't run — the program itself is at fault.
        R::Unexecutable => FailureAction::Permanent,
        // Capacity, expiry, or a post-acceptance revert: the request is dead but
        // the input is fine, so resubmit a fresh request.
        R::Unfulfillable | R::Expired | R::Reverted => FailureAction::RetryFresh,
        // Uncategorized backend failure — conservative terminal.
        R::Other(_) => FailureAction::Permanent,
    }
}

#[cfg(test)]
mod tests {
    use zkaleido::ZkVmError;

    use super::*;

    #[test]
    fn zkvm_network_and_not_ready_resume() {
        assert_eq!(
            classify_zkvm_error(&ZkVmError::NetworkRetryableError("503".into())),
            FailureAction::RetryResume
        );
        assert_eq!(
            classify_zkvm_error(&ZkVmError::ProofNotReady),
            FailureAction::RetryResume
        );
    }

    #[test]
    fn zkvm_generation_and_execution_faults_permanent() {
        assert_eq!(
            classify_zkvm_error(&ZkVmError::ProofGenerationError("bad request".into())),
            FailureAction::Permanent
        );
        assert_eq!(
            classify_zkvm_error(&ZkVmError::ExecutionError("panic".into())),
            FailureAction::Permanent
        );
        assert_eq!(
            classify_zkvm_error(&ZkVmError::InvalidELF("bad elf".into())),
            FailureAction::Permanent
        );
        assert_eq!(
            classify_zkvm_error(&ZkVmError::ProofVerificationError("bad proof".into())),
            FailureAction::Permanent
        );
    }

    #[test]
    fn zkvm_other_resumes_conservatively() {
        assert_eq!(
            classify_zkvm_error(&ZkVmError::Other("???".into())),
            FailureAction::RetryResume
        );
    }

    #[cfg(feature = "remote")]
    #[test]
    fn remote_status_failures_classified() {
        use zkaleido::RemoteProofFailureReason as R;
        assert_eq!(
            classify_remote_failure(&R::Unexecutable),
            FailureAction::Permanent
        );
        assert_eq!(
            classify_remote_failure(&R::Unfulfillable),
            FailureAction::RetryFresh
        );
        assert_eq!(
            classify_remote_failure(&R::Expired),
            FailureAction::RetryFresh
        );
        assert_eq!(
            classify_remote_failure(&R::Reverted),
            FailureAction::RetryFresh
        );
        assert_eq!(
            classify_remote_failure(&R::Other("nope".into())),
            FailureAction::Permanent
        );
    }
}
