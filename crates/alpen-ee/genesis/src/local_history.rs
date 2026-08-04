//! Classifies the local EE history retained by a node database.

use alpen_ee_common::{get_batch_anchor, BatchStatus, BatchStorage, ExecBlockStorage, Storage};
use alpen_ee_config::AlpenEeConfig;
use eyre::{bail, ensure, ContextCompat};
use strata_identifiers::EpochCommitment;
use tracing::info;

use crate::{
    ensure_batch_genesis, ensure_finalized_exec_chain_genesis, ensure_genesis_ee_account_state,
};

/// Describes where the locally retained EE history begins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalHistory {
    /// No EE history has been initialized.
    Empty,
    /// Existing history is compatible with normal genesis initialization.
    GenesisAnchored,
    /// All retained histories start after genesis.
    RecoveryAnchored,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ComponentHistory {
    Empty,
    GenesisAnchored,
    RecoveryAnchored,
}

fn classify_components(
    account: ComponentHistory,
    execution: ComponentHistory,
    batch: ComponentHistory,
) -> eyre::Result<LocalHistory> {
    let components = [account, execution, batch];

    if components
        .iter()
        .all(|history| *history == ComponentHistory::Empty)
    {
        return Ok(LocalHistory::Empty);
    }

    if components
        .iter()
        .all(|history| *history != ComponentHistory::RecoveryAnchored)
    {
        // This also permits restart after a normal genesis initialization was interrupted between
        // components. The strict genesis helpers validate and complete the missing records.
        return Ok(LocalHistory::GenesisAnchored);
    }

    if components
        .iter()
        .all(|history| *history == ComponentHistory::RecoveryAnchored)
    {
        return Ok(LocalHistory::RecoveryAnchored);
    }

    bail!(
        "inconsistent local EE history: account={account:?}, execution={execution:?}, \
         batch={batch:?}"
    )
}

/// Inspects existing records without mutating storage.
pub async fn inspect_local_history<TStorage>(
    genesis_epoch: &EpochCommitment,
    storage: &TStorage,
) -> eyre::Result<LocalHistory>
where
    TStorage: Storage + ExecBlockStorage + BatchStorage,
{
    let account = if storage
        .ee_account_state(genesis_epoch.last_blkid().into())
        .await?
        .is_some()
    {
        ComponentHistory::GenesisAnchored
    } else if storage.best_ee_account_state().await?.is_some() {
        ComponentHistory::RecoveryAnchored
    } else {
        ComponentHistory::Empty
    };

    let execution = if storage.get_finalized_block_at_height(0).await?.is_some() {
        ComponentHistory::GenesisAnchored
    } else if storage.best_finalized_block().await?.is_some() {
        ComponentHistory::RecoveryAnchored
    } else {
        ComponentHistory::Empty
    };

    let batch = match get_batch_anchor(storage).await? {
        Some((anchor, _)) if anchor.idx() == 0 => ComponentHistory::GenesisAnchored,
        Some(_) => ComponentHistory::RecoveryAnchored,
        None => ComponentHistory::Empty,
    };

    classify_components(account, execution, batch)
}

/// Checks that the existing sparse records provide the minimum retained history needed at startup.
pub async fn validate_recovered_local_history<TStorage>(
    genesis_epoch: &EpochCommitment,
    storage: &TStorage,
) -> eyre::Result<()>
where
    TStorage: Storage + ExecBlockStorage + BatchStorage,
{
    let account = storage
        .best_ee_account_state()
        .await?
        .context("recovered EE history has no account-state anchor")?;
    ensure!(
        account.ol_epoch() > genesis_epoch.epoch(),
        "recovered EE account-state anchor must be after genesis"
    );

    let finalized = storage
        .best_finalized_block()
        .await?
        .context("recovered EE history has no finalized execution anchor")?;
    ensure!(
        finalized.blocknum() > 0,
        "recovered finalized execution anchor must be after genesis"
    );

    let (batch, status) = get_batch_anchor(storage)
        .await?
        .context("recovered EE history has no batch anchor")?;
    ensure!(
        batch.idx() > 0,
        "recovered batch anchor must be after genesis"
    );
    ensure!(
        matches!(status, BatchStatus::Genesis),
        "recovered batch anchor must have genesis status"
    );

    let recovered_tip = storage
        .get_exec_block(batch.last_block())
        .await?
        .context("recovered batch anchor end block is missing from execution storage")?;
    ensure!(
        recovered_tip.blocknum() == batch.last_blocknum(),
        "recovered batch anchor end block number does not match execution storage"
    );

    info!(
        account_epoch = account.ol_epoch(),
        finalized_height = finalized.blocknum(),
        batch_idx = batch.idx(),
        recovered_tip = %batch.last_block(),
        "validated sparse local EE history"
    );

    Ok(())
}

/// Ensures normal genesis history or validates an already materialized sparse recovery history.
pub async fn ensure_local_history<TStorage>(
    config: &AlpenEeConfig,
    genesis_epoch: &EpochCommitment,
    storage: &TStorage,
) -> eyre::Result<LocalHistory>
where
    TStorage: Storage + ExecBlockStorage + BatchStorage,
{
    let history = inspect_local_history(genesis_epoch, storage).await?;

    match history {
        LocalHistory::Empty | LocalHistory::GenesisAnchored => {
            ensure_genesis_ee_account_state(config, genesis_epoch, storage).await?;
            ensure_finalized_exec_chain_genesis(
                config,
                genesis_epoch.to_block_commitment(),
                storage,
            )
            .await?;
            ensure_batch_genesis(config, storage).await?;
        }
        LocalHistory::RecoveryAnchored => {
            validate_recovered_local_history(genesis_epoch, storage).await?;
        }
    }

    Ok(history)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_components_are_empty_history() {
        assert_eq!(
            classify_components(
                ComponentHistory::Empty,
                ComponentHistory::Empty,
                ComponentHistory::Empty,
            )
            .unwrap(),
            LocalHistory::Empty
        );
    }

    #[test]
    fn partial_normal_genesis_is_genesis_anchored() {
        assert_eq!(
            classify_components(
                ComponentHistory::GenesisAnchored,
                ComponentHistory::Empty,
                ComponentHistory::Empty,
            )
            .unwrap(),
            LocalHistory::GenesisAnchored
        );
    }

    #[test]
    fn all_sparse_components_are_recovery_anchored() {
        assert_eq!(
            classify_components(
                ComponentHistory::RecoveryAnchored,
                ComponentHistory::RecoveryAnchored,
                ComponentHistory::RecoveryAnchored,
            )
            .unwrap(),
            LocalHistory::RecoveryAnchored
        );
    }

    #[test]
    fn mixed_genesis_and_sparse_components_are_rejected() {
        assert!(classify_components(
            ComponentHistory::RecoveryAnchored,
            ComponentHistory::GenesisAnchored,
            ComponentHistory::RecoveryAnchored,
        )
        .is_err());
    }
}
