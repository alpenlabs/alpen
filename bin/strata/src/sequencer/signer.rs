//! Internal sequencer signer worker.

use std::time::Duration;

use anyhow::{Result, anyhow};
use tokio::sync::mpsc;
use tracing::info;

use super::{
    duty_executor::duty_executor_worker, duty_fetcher::duty_fetcher_worker, helpers::load_seqkey,
};
use crate::{args::Args, run_context::RunContext};

/// Default duty poll interval in milliseconds.
const DEFAULT_DUTY_POLL_INTERVAL_MS: u64 = 1_000;

/// Starts the sequencer signer worker.
pub(crate) fn start_sequencer_signer(runctx: &RunContext, args: &Args) -> Result<()> {
    // Get the sequencer handles (must be present when running as sequencer).
    let handles = runctx
        .sequencer_handles()
        .ok_or_else(|| anyhow!("sequencer handles not available (is_sequencer=true required)"))?;

    // Get the sequencer key path.
    let Some(sequencer_key_path) = args.sequencer_key.as_ref() else {
        return Err(anyhow!(
            "--sequencer-key is required when --sequencer is set"
        ));
    };

    // Load the sequencer key.
    let sequencer_key = load_seqkey(sequencer_key_path)?;

    // Get the duty poll interval.
    let poll_interval_ms = args
        .duty_poll_interval
        .unwrap_or(DEFAULT_DUTY_POLL_INTERVAL_MS);

    // Create a channel for duties.
    let (duty_tx, duty_rx) = mpsc::channel(64);

    // Spawn the duty fetcher worker.
    runctx.executor().spawn_critical_async(
        "sequencer-duty-fetcher",
        duty_fetcher_worker(
            handles.template_manager().clone(),
            runctx.storage().clone(),
            runctx.status_channel().clone(),
            duty_tx,
            Duration::from_millis(poll_interval_ms),
        ),
    );

    // Spawn the duty executor worker.
    runctx.executor().spawn_critical_async(
        "sequencer-duty-executor",
        duty_executor_worker(
            handles.template_manager().clone(),
            handles.envelope_handle().clone(),
            runctx.storage().clone(),
            runctx.fcm_handle().clone(),
            duty_rx,
            runctx.task_manager().handle().clone(),
            sequencer_key.sk,
        ),
    );

    info!(%poll_interval_ms, "Sequencer signer started");

    Ok(())
}
