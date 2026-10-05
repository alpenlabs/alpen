//! Configuration for the Alpen codebase.

mod asm_execution;
pub mod btcio;
mod config;

pub use asm_execution::{AsmExecutionParams, AsmExecutionTarget};
pub use config::*;
