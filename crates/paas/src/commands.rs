//! Command messages for PaaS service

use strata_primitives::proof::ProofContext;
use strata_service::CommandCompletionSender;
use uuid::Uuid;

use crate::{PaaSError, PaaSReport, TaskStatus};

/// Task identifier (UUID v4)
pub type TaskId = Uuid;

/// Command messages for interacting with PaaS service
#[derive(Debug)]
pub enum PaaSCommand {
    /// Create a new proof task
    CreateTask {
        /// Proof context to generate
        context: ProofContext,
        /// Dependencies that must complete first
        deps: Vec<ProofContext>,
        /// Completion channel for response
        completion: CommandCompletionSender<Result<TaskId, PaaSError>>,
    },

    /// Get the status of a proof task
    GetTaskStatus {
        /// Task identifier
        task_id: TaskId,
        /// Completion channel for response
        completion: CommandCompletionSender<Result<TaskStatus, PaaSError>>,
    },

    /// Get a completed proof
    GetProof {
        /// Task identifier
        task_id: TaskId,
        /// Completion channel for response
        completion: CommandCompletionSender<Result<Option<ProofData>, PaaSError>>,
    },

    /// Cancel a pending or in-progress task
    CancelTask {
        /// Task identifier
        task_id: TaskId,
        /// Completion channel for response
        completion: CommandCompletionSender<Result<(), PaaSError>>,
    },

    /// Get service metrics report
    GetReport {
        /// Completion channel for response
        completion: CommandCompletionSender<Result<PaaSReport, PaaSError>>,
    },
}

/// Proof data returned from GetProof command
#[derive(Debug, Clone)]
pub struct ProofData {
    /// The proof receipt
    pub receipt: Vec<u8>,
    /// Public values (optional)
    pub public_values: Option<Vec<u8>>,
    /// Verification key (optional)
    pub verification_key: Option<Vec<u8>>,
}
