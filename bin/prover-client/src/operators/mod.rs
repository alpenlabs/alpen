//! A module defining operations for proof generation using ZKVMs.
//!
//! This module provides operators that encapsulate RPC client accessors
//! for fetching data needed for proof generation.
//!
//! NOTE: The original ProvingOp trait and task creation methods have been removed
//! as they are now handled by the PaaS (Prover-as-a-Service) framework.
//! This module now only contains minimal accessor methods for RPC clients.
//!
//! Supported ZKVMs:
//!
//! - Native
//! - SP1 (requires `sp1` feature enabled)

pub(crate) mod checkpoint;
pub(crate) mod cl_stf;
pub(crate) mod evm_ee;
pub(crate) mod operator;

pub(crate) use operator::ProofOperator;

