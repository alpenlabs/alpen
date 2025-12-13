//! Integration tests for ASM subprotocols.
//!
//! This crate contains integration tests that require external dependencies
//! like Bitcoin regtest nodes. Tests are organized by subprotocol.

#[cfg(test)]
mod subprotocols;
pub mod test_data;
pub mod test_env;
pub mod worker_context;
