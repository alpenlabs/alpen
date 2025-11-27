//! Timer service for delayed task execution
//!
//! This module provides a channel-based timer service that handles delayed
//! task execution without direct tokio dependencies in business logic.

use std::time::Duration;

use strata_tasks::TaskExecutor;
use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::{state::ProverServiceState, task::TaskId, ProgramType};

/// Commands that can be sent to the timer service
pub enum TimerCommand<P: ProgramType> {
    /// Schedule a retry after a delay
    ScheduleRetry {
        task_id: TaskId<P>,
        delay_secs: u64,
        /// We need to pass state since timer service is decoupled
        /// In the future, we could use a command channel back to state instead
        state: ProverServiceState<P>,
    },
}

/// Timer service that handles delayed task execution
///
/// This service runs as a background task and processes timer commands,
/// spawning delayed executions on the TaskExecutor.
pub struct TimerService<P: ProgramType> {
    receiver: mpsc::UnboundedReceiver<TimerCommand<P>>,
    executor: TaskExecutor,
}

impl<P: ProgramType> TimerService<P> {
    /// Create a new timer service
    pub fn new(
        receiver: mpsc::UnboundedReceiver<TimerCommand<P>>,
        executor: TaskExecutor,
    ) -> Self {
        Self { receiver, executor }
    }

    /// Run the timer service loop
    ///
    /// This method processes timer commands and spawns delayed tasks.
    /// It runs until the channel is closed.
    pub async fn run(mut self) {
        debug!("Timer service started");

        while let Some(cmd) = self.receiver.recv().await {
            match cmd {
                TimerCommand::ScheduleRetry {
                    task_id,
                    delay_secs,
                    state,
                } => {
                    debug!(?task_id, delay_secs, "Scheduling retry");

                    let executor = self.executor.clone();
                    // Spawn the delayed task as non-critical background work
                    executor.handle().spawn(async move {
                        // TODO: This still uses tokio::time::sleep, but it's isolated here
                        // In the future, we could use a more abstract delay mechanism
                        tokio::time::sleep(Duration::from_secs(delay_secs)).await;
                        debug!(?task_id, "Retry delay elapsed, executing task");
                        state.execute_and_track(task_id).await;
                    });
                }
            }
        }

        warn!("Timer service stopped (channel closed)");
    }
}

/// Handle for sending timer commands
#[derive(Clone)]
pub struct TimerHandle<P: ProgramType> {
    sender: mpsc::UnboundedSender<TimerCommand<P>>,
}

impl<P: ProgramType> TimerHandle<P> {
    /// Create a new timer handle
    pub fn new(sender: mpsc::UnboundedSender<TimerCommand<P>>) -> Self {
        Self { sender }
    }

    /// Schedule a retry for a task
    pub fn schedule_retry(
        &self,
        task_id: TaskId<P>,
        delay_secs: u64,
        state: ProverServiceState<P>,
    ) {
        if let Err(e) = self.sender.send(TimerCommand::ScheduleRetry {
            task_id,
            delay_secs,
            state,
        }) {
            // This should only happen if timer service is stopped
            tracing::error!(?e, "Failed to send timer command (service stopped?)");
        }
    }
}
