//! EE proof generation module.
//!
//! Implements [`BatchProver`] using the PaaS framework, orchestrating a two-stage
//! proof pipeline (chunk proofs -> account proof) for EE batch proving.

mod batch_prover;
mod host_resolver;
mod input_fetcher;
mod orchestrator;
mod proof_store;
mod task_store;

pub(crate) use batch_prover::PaasBatchProver;
pub(crate) use host_resolver::EeHostResolver;
pub(crate) use input_fetcher::{AcctInputFetcher, ChunkInputFetcher};
pub(crate) use proof_store::ProofStore;
pub(crate) use task_store::SledTaskStore;
