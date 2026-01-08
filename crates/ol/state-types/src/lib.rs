//! All the types definitions for OL.

// Will be required in the future.
use ssz as _;

mod account;
mod epochal;
mod global;
mod ledger;
mod serial_map;
mod snark_account;
mod state_provider;
mod toplevel;
mod write_batch;

pub use account::*;
pub use epochal::*;
pub use global::*;
pub use ledger::*;
pub use serial_map::*;
pub use snark_account::*;
pub use state_provider::*;
pub use toplevel::*;
pub use write_batch::*;
