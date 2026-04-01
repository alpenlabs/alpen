//! A module that contains the input, handlers, state and context for CheckpointSync service.

mod context;
mod input;
mod service;
mod state;

pub use context::{BitcoinDAExtractor, CheckpointSyncCtxImpl};
pub use service::{CssServiceHandle, CheckpointSyncService, CheckpointSyncStatus, start_css_service};
pub use state::CheckpointSyncState;
