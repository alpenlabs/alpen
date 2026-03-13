//! Internal sequencer signer service launcher.

use std::{sync::Arc, time::Duration};

use anyhow::{Result, anyhow};
use strata_ol_sequencer::{SequencerBuilder, SequencerServiceStatus};
use strata_service::ServiceMonitor;
use tracing::info;
use zeroize::Zeroize;

use super::{helpers::load_seqkey, node_context::NodeSequencerContext};
use crate::{args::Args, run_context::RunContext};

/// Starts the sequencer signer service.
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

    let sequencer_config = runctx
        .config()
        .sequencer
        .as_ref()
        .ok_or_else(|| anyhow!("sequencer config not available"))?;
    let poll_interval_ms = sequencer_config.ol_block_time_ms;

    let context = Arc::new(NodeSequencerContext::new(
        handles.blockasm_handle().clone(),
        handles.envelope_handle().clone(),
        runctx.storage().clone(),
        runctx.fcm_handle().clone(),
        runctx.status_channel().clone(),
    ));

    let launch_result = runctx.task_manager().handle().block_on(async {
        SequencerBuilder::new(
            context,
            sequencer_key.sk,
            Duration::from_millis(poll_interval_ms),
        )
        .launch(runctx.executor())
        .await
    });

    // Zeroize the sequencer key.
    sequencer_key.zeroize();

    let service_monitor = launch_result?;

    info!(%poll_interval_ms, "Sequencer signer service started");

    Ok(service_monitor)
}
