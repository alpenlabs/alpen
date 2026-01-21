use super::{account::Storage, exec_block::ExecBlockStorage, BatchStorage};
use crate::{Batch, BatchStatus, EeAccountStateAtEpoch, ExecBlockRecord, StorageError};

/// Gets the latest batch from storage, returning an error if none exists.
///
/// This function enforces the system invariant that at least one batch (the genesis batch)
/// must always exist in storage after initialization via `ensure_batch_genesis`.
///
/// # Usage
///
/// This function should only be called after `ensure_batch_genesis` has completed successfully
/// at application startup. Calling it before initialization will result in an invariant violation
/// error.
///
/// # Errors
///
/// Returns [`StorageError::InvariantViolated`] if no batch exists in storage, indicating either:
/// - The function was called before `ensure_batch_genesis` completed
/// - Critical storage corruption or initialization failure
///
/// Returns other [`StorageError`] variants for underlying storage failures.
///
/// `ensure_batch_genesis`: alpen_ee_genesis::batch::ensure_batch_genesis
pub async fn require_latest_batch(
    storage: &impl BatchStorage,
) -> Result<(Batch, BatchStatus), StorageError> {
    storage.get_latest_batch().await?.ok_or_else(|| {
        StorageError::invariant_violated("no batch exists in storage after genesis initialization")
    })
}

/// Gets the best EE account state from storage, returning an error if none exists.
///
/// This function enforces the system invariant that at least one EE account state
/// must always exist in storage after initialization via `ensure_genesis_ee_account_state`.
///
/// # Usage
///
/// This function should only be called after `ensure_genesis_ee_account_state` has completed
/// successfully at application startup. Calling it before initialization will result in an
/// invariant violation error.
///
/// # Errors
///
/// Returns [`StorageError::InvariantViolated`] if no EE account state exists in storage, indicating
/// either:
/// - The function was called before `ensure_genesis_ee_account_state` completed
/// - Critical storage corruption or initialization failure
///
/// Returns other [`StorageError`] variants for underlying storage failures.
///
/// `ensure_genesis_ee_account_state`:
/// alpen_ee_genesis::account_state::ensure_genesis_ee_account_state
pub async fn require_best_ee_account_state(
    storage: &impl Storage,
) -> Result<EeAccountStateAtEpoch, StorageError> {
    storage.best_ee_account_state().await?.ok_or_else(|| {
        StorageError::invariant_violated(
            "no EE account state exists in storage after genesis initialization",
        )
    })
}

/// Gets the best finalized block from storage, returning an error if none exists.
///
/// This function enforces the system invariant that at least one finalized block
/// must always exist in storage after initialization via `ensure_finalized_exec_chain_genesis`.
///
/// # Usage
///
/// This function should only be called after `ensure_finalized_exec_chain_genesis` has completed
/// successfully at application startup. Calling it before initialization will result in an
/// invariant violation error.
///
/// # Errors
///
/// Returns [`StorageError::InvariantViolated`] if no finalized block exists in storage, indicating
/// either:
/// - The function was called before `ensure_finalized_exec_chain_genesis` completed
/// - Critical storage corruption or initialization failure
///
/// Returns other [`StorageError`] variants for underlying storage failures.
///
/// `ensure_finalized_exec_chain_genesis`:
/// alpen_ee_genesis::exec_chain::ensure_finalized_exec_chain_genesis
pub async fn require_best_finalized_block(
    storage: &impl ExecBlockStorage,
) -> Result<ExecBlockRecord, StorageError> {
    storage.best_finalized_block().await?.ok_or_else(|| {
        StorageError::invariant_violated(
            "no finalized block exists in storage after genesis initialization",
        )
    })
}
