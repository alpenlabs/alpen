//! Block template management for block assembly.

mod block_assembly;
mod error;
mod handle;
mod types;
mod worker;

pub use block_assembly::prepare_block;
pub use error::Error;
pub use handle::{TemplateManagerHandle, TemplateManagerRequest};
pub use types::{BlockCompletionData, BlockGenerationConfig, BlockTemplate, FullBlockTemplate};
pub use worker::{worker_task, SharedState, WorkerContext};
