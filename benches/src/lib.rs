//! Benchmarks for Alpen database implementations.
//!
//! This crate contains benchmarks for various implementations.

// Import dependencies to satisfy linting
#[allow(unused_imports)]
use arbitrary as _;
#[allow(unused_imports)]
use bitcoin as _;
#[allow(unused_imports)]
use criterion as _;
#[allow(unused_imports)]
use strata_db as _;
#[allow(unused_imports)]
use strata_primitives as _;
#[allow(unused_imports)]
use strata_state as _;

pub mod db;
