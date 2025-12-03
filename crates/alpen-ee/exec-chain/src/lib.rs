#![expect(unused_crate_dependencies, reason = "wip")]
//! Maintain in memory view of canonical exec chain.

mod handle;
mod orphan_tracker;
mod state;
mod task;
mod unfinalized_tracker;

pub use handle::{build_task, ExecChainHandle};
pub use state::{init_exec_chain_state_from_storage, ExecChainState};
