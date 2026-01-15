//! Types relating to snark accounts and the snark account proof interface.

mod accumulators;
mod error;
mod messages;
mod outputs;
mod proof_interface;
mod state;
mod update;

// Include generated SSZ types from build.rs output
#[allow(
    clippy::all,
    unreachable_pub,
    clippy::allow_attributes,
    clippy::absolute_paths,
    reason = "generated code"
)]
mod ssz_generated {
    include!(concat!(env!("OUT_DIR"), "/generated.rs"));
}

pub use error::OutputsError;
pub use ssz_generated::ssz::{
    accumulators::*, messages::*, outputs::*, proof_interface::*, state::*, update::*,
};
pub use state::Seqno;
