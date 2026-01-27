//! All the types definitions for OL.

// Will be required in the future.
use ssz as _;

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

mod account;
mod batch_application;
mod epochal;
mod global;
mod ledger;
mod serial_map;
mod snark_account;
mod state_provider;
mod toplevel;
mod write_batch;

#[cfg(test)]
mod test_utils;

// Re-export SSZ-generated types that are used publicly
pub use batch_application::*;
pub use serial_map::*;
pub use ssz_generated::ssz::state::*;
pub use state_provider::*;
pub use write_batch::*;
