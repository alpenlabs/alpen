//! Types relating to snark accounts and the snark account proof interface.

#![expect(unused, reason = "in development")] // in-development

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
pub use state::{ProofState, SnarkAccountState};
pub use update::*;
