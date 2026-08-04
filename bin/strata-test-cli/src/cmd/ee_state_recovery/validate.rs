//! Preflight and post-materialization checks for recovery output.

use std::path::Path;

use alloy_primitives::B256;
use alpen_ee_common::{BatchStorage, ExecBlockStorage, Storage};
use alpen_ee_database::init_db_storage;
use anyhow::{anyhow, bail, Context, Result};
use strata_acct_types::Hash;
use strata_snark_acct_runtime::IInnerState;
use tokio::runtime::Handle;

/// Refuses ambiguous, in-place, or overwriting recovery targets.
pub(super) fn validate_paths(source: &Path, output: &Path, staging: &Path) -> Result<()> {
    if !source.is_dir() {
        bail!("source EE datadir does not exist: {}", source.display());
    }
    if output.exists() {
        bail!("output EE datadir already exists: {}", output.display());
    }
    if staging.exists() {
        bail!(
            "recovery staging directory already exists: {}",
            staging.display()
        );
    }
    let source = source
        .canonicalize()
        .with_context(|| format!("canonicalizing source datadir {}", source.display()))?;
    let output_parent = output
        .parent()
        .context("output datadir must have a parent directory")?
        .canonicalize()
        .with_context(|| format!("canonicalizing output parent for {}", output.display()))?;
    let output_name = output
        .file_name()
        .context("output datadir must have a final path component")?;
    if source == output_parent.join(output_name) {
        bail!("source and output EE datadirs must be different");
    }
    Ok(())
}

/// Verifies the sparse Sled frontiers before publishing the datadir.
pub(super) async fn validate_materialized_datadir(
    datadir: &Path,
    target_update_seq_no: u64,
    expected_exec_blkid: B256,
    expected_inner_state_root: B256,
) -> Result<()> {
    let databases = init_db_storage(datadir, 5)
        .map_err(|error| anyhow!("opening recovered datadir {}: {error:#}", datadir.display()))?;
    let storage = databases.node_storage(Handle::current());

    let account = storage
        .best_ee_account_state()
        .await?
        .context("recovered datadir has no EE account-state frontier")?;
    let actual_inner_state_root = B256::from(account.ee_state().compute_state_root().0);
    if actual_inner_state_root != expected_inner_state_root {
        bail!(
            "recovered Sled account root {actual_inner_state_root} does not match OL root \
             {expected_inner_state_root}"
        );
    }
    if B256::from(account.last_exec_blkid().0) != expected_exec_blkid {
        bail!("recovered Sled account state has the wrong execution tip");
    }

    let expected_batch_idx = target_update_seq_no
        .checked_add(1)
        .context("target batch index overflow")?;
    let (batch, _) = storage
        .get_latest_batch()
        .await?
        .context("recovered datadir has no batch anchor")?;
    if batch.idx() != expected_batch_idx || B256::from(batch.last_block().0) != expected_exec_blkid
    {
        bail!("recovered Sled batch anchor does not match the OL recovery target");
    }

    let finalized = storage
        .best_finalized_block()
        .await?
        .context("recovered datadir has no finalized execution anchor")?;
    if finalized.blockhash() != Hash::from(expected_exec_blkid.0) {
        bail!("recovered Sled finalized anchor does not match the OL recovery target");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::validate_paths;

    #[test]
    fn accepts_distinct_nonexistent_output() {
        let root = tempdir().unwrap();
        let source = root.path().join("source");
        let output = root.path().join("recovered");
        let staging = root.path().join(".recovered.recovery-staging");
        fs::create_dir(&source).unwrap();

        validate_paths(&source, &output, &staging).unwrap();
    }

    #[test]
    fn rejects_existing_output() {
        let root = tempdir().unwrap();
        let source = root.path().join("source");
        let output = root.path().join("recovered");
        let staging = root.path().join(".recovered.recovery-staging");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&output).unwrap();

        let error = validate_paths(&source, &output, &staging).unwrap_err();

        assert!(error.to_string().contains("already exists"));
    }

    #[test]
    fn rejects_existing_staging_output() {
        let root = tempdir().unwrap();
        let source = root.path().join("source");
        let output = root.path().join("recovered");
        let staging = root.path().join(".recovered.recovery-staging");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&staging).unwrap();

        let error = validate_paths(&source, &output, &staging).unwrap_err();

        assert!(error.to_string().contains("recovery staging directory"));
    }
}
