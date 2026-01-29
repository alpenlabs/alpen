//! Service state for OL checkpoint builder.

use std::sync::Arc;

use strata_checkpoint_types::EpochSummary;
use strata_checkpoint_types_ssz::{CheckpointPayload, CheckpointSidecar, CheckpointTip};
use strata_db_types::types::OLCheckpointEntry;
use strata_service::ServiceState;
use strata_status::StatusChannel;
use strata_storage::NodeStorage;
use tokio::runtime::Handle;
use tracing::{debug, info, warn};

use crate::{
    errors::{OLCheckpointError, WorkerResult},
    providers::{DaProvider, LogProvider, ProofProvider},
};

#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have Debug implementation"
)]
pub struct OLCheckpointServiceState {
    deps: OLCheckpointDeps,
    initialized: bool,
    last_processed_epoch: Option<u32>,
}

struct OLCheckpointDeps {
    storage: Arc<NodeStorage>,
    status_channel: StatusChannel,
    runtime_handle: Handle,
    da_provider: Arc<dyn DaProvider>,
    log_provider: Arc<dyn LogProvider>,
    proof_provider: Arc<dyn ProofProvider>,
}

impl OLCheckpointServiceState {
    pub fn new(
        storage: Arc<NodeStorage>,
        status_channel: StatusChannel,
        runtime_handle: Handle,
        da_provider: Arc<dyn DaProvider>,
        log_provider: Arc<dyn LogProvider>,
        proof_provider: Arc<dyn ProofProvider>,
    ) -> Self {
        Self {
            deps: OLCheckpointDeps {
                storage,
                status_channel,
                runtime_handle,
                da_provider,
                log_provider,
                proof_provider,
            },
            initialized: false,
            last_processed_epoch: None,
        }
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    pub fn last_processed_epoch(&self) -> Option<u32> {
        self.last_processed_epoch
    }

    pub fn initialize(&mut self) -> anyhow::Result<()> {
        self.wait_for_genesis()?;
        self.initialized = true;
        Ok(())
    }

    pub fn tick(&mut self) -> WorkerResult<()> {
        if !self.initialized {
            return Err(OLCheckpointError::NotInitialized);
        }

        let ol_checkpoint = self.deps.storage.ol_checkpoint();

        let Some(last_epoch) = ol_checkpoint.get_last_summarized_epoch_blocking()? else {
            debug!("no OL epoch summaries available");
            return Ok(());
        };

        let start_epoch = self
            .last_processed_epoch
            .map(|epoch| epoch as u64)
            .unwrap_or(0);

        for epoch_idx in start_epoch..=last_epoch {
            let commitments = ol_checkpoint.get_epoch_commitments_at_blocking(epoch_idx)?;
            if commitments.is_empty() {
                warn!(epoch = epoch_idx, "no epoch commitments found");
                continue;
            }

            if commitments.len() > 1 {
                warn!(
                    epoch = epoch_idx,
                    ignored = commitments.len() - 1,
                    "multiple epoch summaries found, using first"
                );
            }

            let commitment = commitments[0];
            let epoch = commitment.epoch();

            if ol_checkpoint.get_checkpoint_blocking(epoch)?.is_some() {
                continue;
            }

            let summary = ol_checkpoint
                .get_epoch_summary_blocking(commitment)?
                .ok_or(OLCheckpointError::MissingEpochSummary(commitment))?;

            let payload = build_checkpoint_payload(
                &summary,
                self.deps.da_provider.as_ref(),
                self.deps.log_provider.as_ref(),
                self.deps.proof_provider.as_ref(),
            )?;
            let entry = OLCheckpointEntry::new_unsigned(payload);
            ol_checkpoint.put_checkpoint_blocking(epoch, entry)?;

            info!(epoch, "stored OL checkpoint entry");
            self.last_processed_epoch = Some(epoch_idx as u32);
        }

        Ok(())
    }

    fn wait_for_genesis(&self) -> WorkerResult<()> {
        self.deps
            .runtime_handle
            .block_on(self.deps.status_channel.wait_until_genesis())
            .map_err(|err| OLCheckpointError::StatusChannel(err.to_string()))?;
        Ok(())
    }
}

impl ServiceState for OLCheckpointServiceState {
    fn name(&self) -> &str {
        "ol_checkpoint"
    }
}

fn build_checkpoint_payload(
    summary: &EpochSummary,
    da_provider: &dyn DaProvider,
    log_provider: &dyn LogProvider,
    proof_provider: &dyn ProofProvider,
) -> WorkerResult<CheckpointPayload> {
    let l1_height = summary.new_l1().height_u32();
    let l2_commitment = *summary.terminal();
    let new_tip = CheckpointTip::new(summary.epoch(), l1_height, l2_commitment);

    let state_bytes = da_provider.compute_state_data(summary)?;
    let ol_logs = log_provider.get_epoch_logs(summary)?;

    let sidecar = CheckpointSidecar::new(state_bytes, ol_logs)
        .map_err(|err| OLCheckpointError::Unexpected(err.to_string()))?;
    let proof = proof_provider.get_proof(summary)?;

    CheckpointPayload::new(new_tip, sidecar, proof)
        .map_err(|err| OLCheckpointError::Unexpected(err.to_string()))
}
