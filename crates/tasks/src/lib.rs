//! Task management.

mod manager;
mod pending_tasks;
mod print_panic;
mod shutdown;

pub use manager::{TaskError, TaskExecutor, TaskManager};
pub use print_panic::set_panic_hook;
pub use shutdown::{ShutdownGuard, ShutdownSignal};
