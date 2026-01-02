//! Retry scheduler for delayed task execution
//!
//! This module provides a channel-based retry scheduler that handles delayed
//! task execution using proper command-based communication back to the service.

use std::{sync::Arc, time::Duration};

use strata_service::CommandHandle;
use strata_tasks::TaskExecutor;
use tokio::{sync::mpsc, time::sleep};
use tracing::{debug, warn};

use crate::{program::ProgramType, service::commands::ProverCommand, task::TaskId};

/// Commands that can be sent to the retry scheduler
#[derive(Debug)]
pub enum SchedulerCommand<P: ProgramType> {
    /// Schedule a retry after a delay
    ScheduleRetry { task_id: TaskId<P>, delay_secs: u64 },
}

/// Retry scheduler that handles delayed task execution
///
/// This service runs as a background task and processes scheduler commands,
/// sending retry commands back to the ProverService after delays.
pub(crate) struct RetryScheduler<P: ProgramType> {
    receiver: mpsc::UnboundedReceiver<SchedulerCommand<P>>,
    command_handle: Arc<CommandHandle<ProverCommand<TaskId<P>>>>,
    executor: TaskExecutor,
}

impl<P: ProgramType> RetryScheduler<P> {
    /// Create a new retry scheduler
    pub(crate) fn new(
        receiver: mpsc::UnboundedReceiver<SchedulerCommand<P>>,
        command_handle: Arc<CommandHandle<ProverCommand<TaskId<P>>>>,
        executor: TaskExecutor,
    ) -> Self {
        Self {
            receiver,
            command_handle,
            executor,
        }
    }

    /// Run the retry scheduler loop
    ///
    /// This method processes scheduler commands and spawns delayed tasks
    /// that send retry commands back to the service.
    pub(crate) async fn run(mut self) {
        debug!("Retry scheduler started");

        while let Some(cmd) = self.receiver.recv().await {
            match cmd {
                SchedulerCommand::ScheduleRetry {
                    task_id,
                    delay_secs,
                } => {
                    debug!(?task_id, delay_secs, "Scheduling retry");

                    let command_handle = Arc::clone(&self.command_handle);
                    // Spawn the delayed task as non-critical background work
                    self.executor.handle().spawn(async move {
                        // TODO: This still uses tokio::time::sleep, but it's isolated here
                        // In the future, we could use a more abstract delay mechanism
                        sleep(Duration::from_secs(delay_secs)).await;
                        debug!(?task_id, "Retry delay elapsed, sending retry command");

                        // Send retry command back to service (fire and forget)
                        let _ = command_handle
                            .send(ProverCommand::RetryTask { task_id })
                            .await;
                    });
                }
            }
        }

        warn!("Retry scheduler stopped (channel closed)");
    }
}

/// Handle for sending scheduler commands
#[derive(Clone, Debug)]
pub struct SchedulerHandle<P: ProgramType> {
    sender: mpsc::UnboundedSender<SchedulerCommand<P>>,
}

impl<P: ProgramType> SchedulerHandle<P> {
    /// Create a new scheduler handle
    pub fn new(sender: mpsc::UnboundedSender<SchedulerCommand<P>>) -> Self {
        Self { sender }
    }

    /// Schedule a retry for a task
    ///
    /// The scheduler will send a RetryTask command back to the service after the delay.
    pub fn schedule_retry(&self, task_id: TaskId<P>, delay_secs: u64) {
        if let Err(e) = self.sender.send(SchedulerCommand::ScheduleRetry {
            task_id,
            delay_secs,
        }) {
            // This should only happen if retry scheduler is stopped
            tracing::error!(?e, "Failed to send scheduler command (service stopped?)");
        }
    }
}
