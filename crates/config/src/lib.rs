//! Configuration for the Alpen codebase.

mod asm_execution;
pub use asm_execution::{AsmExecutionParams, AsmExecutionTarget};

pub mod btcio;
mod config;

pub use config::*;
