//! Benchmarks for Alpen database implementations.
//!
//! This crate contains benchmarks for various implementations.

#[allow(
    unused_imports,
    clippy::allow_attributes,
    reason = "used for benchmarking"
)]
use arbitrary as _;
#[allow(
    unused_imports,
    clippy::allow_attributes,
    reason = "used for benchmarking"
)]
use bitcoin as _;
#[allow(
    unused_imports,
    clippy::allow_attributes,
    reason = "used for benchmarking"
)]
use criterion as _;
#[cfg(feature = "db")]
#[allow(
    unused_imports,
    clippy::allow_attributes,
    reason = "used for benchmarking"
)]
use strata_asm_types as _;
#[cfg(feature = "db")]
#[allow(
    unused_imports,
    clippy::allow_attributes,
    reason = "used for benchmarking"
)]
use strata_db as _;
#[cfg(feature = "db")]
#[allow(
    unused_imports,
    clippy::allow_attributes,
    reason = "used for benchmarking"
)]
use strata_ol_chain_types as _;
#[cfg(feature = "db")]
#[allow(
    unused_imports,
    clippy::allow_attributes,
    reason = "used for benchmarking"
)]
use strata_primitives as _;
#[cfg(feature = "db")]
#[allow(
    unused_imports,
    clippy::allow_attributes,
    reason = "used for benchmarking"
)]
use strata_state as _;
#[cfg(feature = "db")]
#[allow(
    unused_imports,
    clippy::allow_attributes,
    reason = "used for benchmarking"
)]
use tempfile as _;

#[cfg(feature = "db")]
pub mod db;
