//! Benchmarks for Alpen database implementations.
//!
//! This crate contains benchmarks for various implementations.

#[allow(unused_imports)]
use arbitrary as _;
#[allow(unused_imports)]
use bitcoin as _;
#[allow(unused_imports)]
use criterion as _;
#[cfg(feature = "db")]
#[allow(unused_imports)]
use strata_asm_types as _;
#[cfg(feature = "db")]
#[allow(unused_imports)]
use strata_db as _;
#[cfg(feature = "db")]
#[allow(unused_imports)]
use strata_primitives as _;
#[cfg(feature = "db")]
#[allow(unused_imports)]
use strata_state as _;
#[cfg(feature = "db")]
#[allow(unused_imports)]
use tempfile as _;

#[cfg(feature = "db")]
pub mod db;
