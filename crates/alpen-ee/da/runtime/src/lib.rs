//! Proof-side data availability for the Alpen EE.
//!
//! Pairs DA-witness construction with DA-witness verification in one crate (the
//! producer and verifier of the same artifact), kept free of Bitcoin-RPC / node
//! storage / async dependencies so it links cleanly into ZKVM guest builds.
//!
//! - [`verification`] — the DA witness verifier consumed by `strata-proofimpl-alpen-acct`.
//!   Reassembles a posted blob from witnessed commit/reveal transactions, checks magic / version,
//!   and ties the result to the active update's public parameters and chunk transitions. Pulls in
//!   the Reth EVM execution stack via `strata-evm-ee`.
//! - [`builders`] (feature `builders`, off by default). host-side helpers that construct the
//!   [`DaWitness`](alpen_ee_da_types::DaWitness) the verifier checks. Enabled by the prover; left
//!   out of guest builds.

pub mod verification;

#[cfg(feature = "builders")]
pub mod builders;
