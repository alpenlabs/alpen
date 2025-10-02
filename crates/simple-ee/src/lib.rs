//! Simple execution environment for testing and demonstration.
//!
//! This provides a simple account-based execution model where "accounts" are
//! identified by SubjectId and have a balance. Transactions can move value
//! between accounts and emit outputs.

mod execution;
mod types;

// Re-export execution environment
pub use execution::SimpleExecutionEnvironment;
// Re-export all types
pub use types::{
    SimpleBlock, SimpleBlockBody, SimpleHeader, SimpleHeaderIntrinsics, SimplePartialState,
    SimpleTransaction, SimpleWriteBatch,
};
