//! Consumer-facing handle wrapping the SF command channel.

use std::{fmt, sync::Arc};

use strata_prover_core::{
    ProofReceiptWithMetadata, ProofSpec, Prover, ProverError, ProverResult, TaskResult, TaskStatus,
};
use strata_service::{CommandHandle, ServiceMonitor};

use crate::service::{Cmd, ProverServiceStatus};

/// Handle for interacting with a running prover service.
///
/// Three methods covering two usage patterns:
/// - **Sequential**: `execute(task)` — submit + block (OL checkpoint)
/// - **Fan-out**: `submit(task)` + `wait_for_tasks(tasks)` (EE pipeline)
#[derive(Clone)]
pub struct ProverHandle<H: ProofSpec> {
    cmd: Arc<CommandHandle<Cmd<H::Task>>>,
    #[expect(dead_code, reason = "monitor held for future health check API")]
    monitor: ServiceMonitor<ProverServiceStatus>,
    prover: Arc<Prover<H>>,
}

impl<H: ProofSpec> fmt::Debug for ProverHandle<H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProverHandle").finish()
    }
}

impl<H: ProofSpec> ProverHandle<H> {
    pub(crate) fn new(
        cmd: Arc<CommandHandle<Cmd<H::Task>>>,
        monitor: ServiceMonitor<ProverServiceStatus>,
        prover: Arc<Prover<H>>,
    ) -> Self {
        Self {
            cmd,
            monitor,
            prover,
        }
    }

    /// Register a task, spawn background proving. Idempotent.
    pub async fn submit(&self, task: H::Task) -> ProverResult<()> {
        self.cmd
            .send_and_wait(|c| Cmd::Submit {
                task: task.clone(),
                completion: c,
            })
            .await
            .map_err(|e| ProverError::Internal(e.into()))?
    }

    /// Submit a task and block until it reaches a terminal state.
    pub async fn execute(&self, task: H::Task) -> ProverResult<TaskResult<H::Task>> {
        self.cmd
            .send_and_wait(|c| Cmd::Execute {
                task: task.clone(),
                completion: c,
            })
            .await
            .map_err(|e| ProverError::Internal(e.into()))?
    }

    /// Block until all tasks reach terminal states. Zero-poll, watch-channel based.
    pub async fn wait_for_tasks(
        &self,
        tasks: &[H::Task],
    ) -> ProverResult<Vec<TaskResult<H::Task>>> {
        self.prover.wait_for_tasks(tasks).await
    }

    /// Get a receipt from the receipt store by task.
    ///
    /// Only available if a [`ReceiptStore`](strata_prover_core::ReceiptStore) was
    /// provided to the builder. Returns `Err` if no store is configured.
    pub fn get_receipt(&self, task: &H::Task) -> ProverResult<Option<ProofReceiptWithMetadata>> {
        self.prover.get_receipt(task)
    }

    /// Current task status.
    pub fn get_status(&self, task: &H::Task) -> ProverResult<TaskStatus> {
        self.prover.get_status(task)
    }
}
