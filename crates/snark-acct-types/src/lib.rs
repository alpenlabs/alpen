//! Types relating to snark accounts and the snark account proof interface.
#![allow(unused)] // in-development

use strata_mmr as _; // might need this shortly

mod accumulators;
mod messages;
mod outputs;
mod proof_interface;
mod state;
mod update;

pub use accumulators::*;
pub use messages::*;
pub use outputs::*;
pub use proof_interface::UpdateProofPubParams;
pub use state::{ProofState, SnarkAcctState};
pub use update::*;
