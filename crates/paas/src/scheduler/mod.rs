//! Retry scheduler for delayed task execution
//!
//! Handles retry scheduling with exponential backoff, coordinating
//! with the main service via command-based communication.

mod service;

pub(crate) use service::{RetryScheduler, SchedulerCommand, SchedulerHandle};
