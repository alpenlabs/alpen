//! ZKVM host statics for the Alpen codebase.
//!
//! Consumers pick a host at prover-build time via
//! `ProverBuilder::.native(Program::native_host())` or `.remote(HOST.clone())`.
//! The prover-client-era dispatcher (which took a `ProofKey` and switched on
//! its `ProofContext` / `ProofZkVm`) is gone — the integrated prover knows
//! its host at build time, no runtime dispatch needed.

#[cfg(feature = "sp1")]
pub mod sp1;
