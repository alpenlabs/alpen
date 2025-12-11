//! Test utilities for ASM (Alpen State Machine) subprotocols.
//!
//! This crate provides testing infrastructure for ASM subprotocols including:
//! - Checkpoint subprotocol test helpers
//! - Transaction builders for envelope format
//! - Mock contexts for unit testing
//! - Fixture generators for test data

// Re-export dependencies used in tests
pub use strata_asm_common as asm_common;
pub use strata_l1_envelope_fmt as l1_envelope_fmt;
pub use strata_l1_txfmt as l1_txfmt;

pub mod checkpoint;
