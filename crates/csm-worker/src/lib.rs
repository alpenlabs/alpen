//! # strata-csm-worker
//!
//! The `strata-csm-worker` crate provides a CSM (Client State Machine) listener service
//! that monitors ASM worker status updates and processes checkpoint logs emitted by the
//! checkpoint subprotocol.

mod checkpoint_extract;
mod constants;
mod context;
mod errors;
mod processor;
mod service;
mod state;
mod status;
#[cfg(test)]
mod test_utils;

pub use context::CsmWorkerContext;
pub use errors::{CsmWorkerError, CsmWorkerResult};
pub use service::CsmWorkerService;
pub use state::CsmWorkerState;
pub use status::CsmWorkerStatus;
