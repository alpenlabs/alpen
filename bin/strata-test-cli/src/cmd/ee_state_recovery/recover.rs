//! End-to-end EE recovery command.

use std::{
    fs::{create_dir, create_dir_all, remove_dir_all, rename, File},
    path::{Path, PathBuf},
    process,
    result::Result as StdResult,
};

use alloy_primitives::B256;
use alpen_chainspec::{chain_value_parser, ee_genesis_block_info};
use anyhow::{anyhow, bail, Context, Error, Result as AnyResult};
use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use tokio::runtime::Builder;

use super::{
    bitcoin::hydrate_manifest,
    ol::load_recovery_target,
    reconstruct::{reconstruct, ReconstructConfig},
    reth_import::{import, RethImportConfig},
    sled_bootstrap::{bootstrap, SledBootstrapConfig},
    source::load_source,
    validate::{validate_materialized_datadir, validate_paths},
};

/// Reconstruct EE state and create a fresh sparse sequencer datadir.
#[derive(Debug, PartialEq, FromArgs)]
#[argh(subcommand, name = "ee-state-recover")]
pub struct EeStateRecoverArgs {
    /// stopped source EE datadir used only for finalized DA transaction IDs
    #[argh(option)]
    pub source_datadir: PathBuf,

    /// new EE datadir to publish after all validation succeeds
    #[argh(option)]
    pub output_datadir: PathBuf,

    /// bitcoin JSON-RPC URL with txindex access
    #[argh(option)]
    pub bitcoin_rpc_url: String,

    /// bitcoin JSON-RPC username
    #[argh(option)]
    pub bitcoin_rpc_user: String,

    /// bitcoin JSON-RPC password
    #[argh(option)]
    pub bitcoin_rpc_password: String,

    /// synced OL node HTTP JSON-RPC URL
    #[argh(option)]
    pub ol_rpc_url: String,

    /// EE Snark account ID on OL
    #[argh(option)]
    pub account_id: String,

    /// OL-accepted EE update sequence to reconstruct
    #[argh(option)]
    pub target_update_seq_no: u64,

    /// chain specification name or path
    #[argh(option, default = "String::from(\"testnet\")")]
    pub chain: String,

    /// print state-changing DA replay details
    #[argh(switch)]
    pub trace_diffs: bool,
}

pub(crate) fn ee_state_recover(args: EeStateRecoverArgs) -> StdResult<(), DisplayedError> {
    recover(args).internal_error("recover EE state")
}

/// Returns a hidden sibling datadir used to materialize and validate the recovered state before
/// atomically publishing it at the requested output path.
fn recovery_staging_datadir(output: &Path) -> AnyResult<PathBuf> {
    let parent = output
        .parent()
        .context("output datadir must have a parent directory")?;
    let name = output
        .file_name()
        .context("output datadir must have a final path component")?
        .to_string_lossy();
    Ok(parent.join(format!(".{name}.recovery-staging-{}", process::id())))
}

fn recover(args: EeStateRecoverArgs) -> AnyResult<()> {
    let output_parent = args
        .output_datadir
        .parent()
        .context("output datadir must have a parent directory")?;
    create_dir_all(output_parent)
        .with_context(|| format!("creating output parent {}", output_parent.display()))?;
    let staging_datadir = recovery_staging_datadir(&args.output_datadir)?;
    validate_paths(&args.source_datadir, &args.output_datadir, &staging_datadir)?;

    let chain_spec = chain_value_parser(&args.chain)
        .map_err(|error| anyhow!("{error:#}"))
        .context("loading EE chain specification")?;
    let genesis_exec_blkid = ee_genesis_block_info(&chain_spec).blockhash();
    let runtime = Builder::new_multi_thread().enable_all().build()?;
    let (target, manifest) = runtime.block_on(async {
        let target = load_recovery_target(
            &args.ol_rpc_url,
            &args.account_id,
            args.target_update_seq_no,
            genesis_exec_blkid,
        )
        .await?;
        let mut manifest = load_source(&args.source_datadir, args.target_update_seq_no).await?;
        hydrate_manifest(
            &mut manifest,
            &args.bitcoin_rpc_url,
            &args.bitcoin_rpc_user,
            &args.bitcoin_rpc_password,
        )
        .await?;
        Ok::<_, Error>((target, manifest))
    })?;
    drop(runtime);

    create_dir(&staging_datadir).with_context(|| {
        format!(
            "creating recovery staging datadir {}",
            staging_datadir.display()
        )
    })?;
    let artifacts = staging_datadir.join(".recovery-artifacts");
    create_dir(&artifacts)?;
    let manifest_path = artifacts.join("manifest.json");
    let state_dump = artifacts.join("state.jsonl");
    let metadata = artifacts.join("metadata.json");
    let anchor_header = artifacts.join("anchor-header.rlp");
    serde_json::to_writer_pretty(File::create(&manifest_path)?, &manifest)?;

    reconstruct(ReconstructConfig {
        manifest: manifest_path,
        chain: args.chain.clone(),
        last_exec_blkid: target.last_exec_blkid,
        expected_inner_state_root: target.expected_inner_state_root,
        pending_inputs: target.pending_inputs.clone(),
        pending_fincls: target.pending_fincls.clone(),
        target_update_seq_no: args.target_update_seq_no,
        state_dump: state_dump.clone(),
        metadata: metadata.clone(),
        anchor_header: anchor_header.clone(),
        trace_diffs: args.trace_diffs,
    })?;

    import(RethImportConfig {
        datadir: staging_datadir.clone(),
        chain: args.chain,
        state: state_dump,
        without_evm: true,
        header: Some(anchor_header),
        header_hash: Some(target.last_exec_blkid),
    })
    .map_err(|error| anyhow!("{error:#}"))
    .context("materializing reconstructed state into Reth")?;

    bootstrap(SledBootstrapConfig {
        datadir: staging_datadir.clone(),
        verified_metadata: metadata,
        base_verified_metadata: None,
        ol_epoch_history: None,
        ol_epoch: target.finalized_epoch.epoch(),
        ol_slot: target.finalized_epoch.last_slot(),
        ol_block_id: B256::from(*target.finalized_epoch.last_blkid().as_ref()),
        finalized_anchor_ol_slot: None,
        finalized_anchor_ol_block_id: None,
        previous_batch_block_hash: target.previous_batch_block_hash,
        next_inbox_msg_idx: target.next_inbox_msg_idx,
        next_deposit_idx: target.next_deposit_idx,
    })?;

    let runtime = Builder::new_multi_thread().enable_all().build()?;
    runtime.block_on(validate_materialized_datadir(
        &staging_datadir,
        args.target_update_seq_no,
        target.last_exec_blkid,
        target.expected_inner_state_root,
    ))?;
    drop(runtime);

    remove_dir_all(&artifacts)
        .with_context(|| format!("removing temporary artifacts {}", artifacts.display()))?;
    if args.output_datadir.exists() {
        bail!(
            "output datadir appeared during recovery; validated staging output remains at {}",
            staging_datadir.display()
        );
    }
    rename(&staging_datadir, &args.output_datadir).with_context(|| {
        format!(
            "publishing recovered datadir {} as {}",
            staging_datadir.display(),
            args.output_datadir.display()
        )
    })?;

    println!(
        "recovered EE update sequence {} into {}",
        args.target_update_seq_no,
        args.output_datadir.display()
    );
    Ok(())
}
