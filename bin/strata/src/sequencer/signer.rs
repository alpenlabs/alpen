//! Internal sequencer signer worker.

use std::time::Duration;

use anyhow::{Result, anyhow};
use strata_service::ServiceMonitor;
use tracing::info;
use zeroize::Zeroize;

use super::{SequencerBuilder, SequencerServiceStatus, helpers::load_seqkey};
use crate::{args::Args, run_context::RunContext};

/// Default duty poll interval in milliseconds.
const DEFAULT_DUTY_POLL_INTERVAL_MS: u64 = 1_000;

/// Starts the sequencer signer worker.
pub(crate) fn start_sequencer_signer(
    runctx: &RunContext,
    args: &Args,
) -> Result<ServiceMonitor<SequencerServiceStatus>> {
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
    let mut sequencer_key = load_seqkey(sequencer_key_path)?;

    // Get the duty poll interval.
    let poll_interval_ms = args
        .duty_poll_interval
        .unwrap_or(DEFAULT_DUTY_POLL_INTERVAL_MS);

    let monitor = runctx.task_manager().handle().block_on(
        SequencerBuilder::new(
            handles.blockasm_handle().clone(),
            handles.envelope_handle().clone(),
            runctx.storage().clone(),
            runctx.fcm_handle().clone(),
            runctx.status_channel().clone(),
            sequencer_key.sk,
            Duration::from_millis(poll_interval_ms),
        )
        .launch(runctx.executor()),
    )?;

    // Zeroize the sequencer key.
    sequencer_key.zeroize();

    info!(%poll_interval_ms, "Sequencer signer started");

    Ok(monitor)
}
