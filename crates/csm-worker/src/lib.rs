//! # strata-csm-worker
//!
//! The `strata-csm-worker` crate provides a CSM (Client State Machine) listener service
//! that monitors ASM worker status updates and processes checkpoint logs emitted by the
//! checkpoint subprotocol.

mod constants;
mod processor;
mod service;
mod state;
mod status;

pub use service::CsmWorkerService;
pub use state::CsmWorkerState;
pub use status::CsmWorkerStatus;
