//! Prover-as-a-Service: Service Framework wrapper for `strata-prover-core`.

mod handle;
mod service;

pub use handle::ProverHandle;
pub use service::{ProverServiceBuilder, ProverServiceStatus};
pub use strata_prover_core::*;
