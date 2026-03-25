//! Block template generation service launcher.

use std::{sync::Arc, time::Duration};

use anyhow::{Result, anyhow};
use strata_ol_sequencer::{SequencerBuilder, SequencerServiceStatus};
use strata_service::ServiceMonitor;
use tracing::info;

use super::node_context::NodeSequencerContext;
use crate::run_context::RunContext;

/// Starts the block production service (template generation only).
pub(crate) fn start_block_producer(
    runctx: &RunContext,
) -> Result<ServiceMonitor<SequencerServiceStatus>> {
    let handles = runctx
        .sequencer_handles()
        .ok_or_else(|| anyhow!("sequencer handles not available (is_sequencer=true required)"))?;

    let ol_block_interval_ms = runctx
        .config()
        .sequencer
        .as_ref()
        .ok_or_else(|| anyhow!("sequencer config required when block producer is enabled"))?
        .ol_block_time_ms;

    let context = Arc::new(NodeSequencerContext::new(
        handles.blockasm_handle().clone(),
        runctx.storage().clone(),
        runctx.status_channel().clone(),
        ol_block_interval_ms,
    ));

    let service_monitor = runctx.task_manager().handle().block_on(async {
        SequencerBuilder::new(context, Duration::from_millis(ol_block_interval_ms))
            .launch(runctx.executor())
            .await
    })?;

    info!(%ol_block_interval_ms, "block producer service started");

    Ok(service_monitor)
}
