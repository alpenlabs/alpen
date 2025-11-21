//! All the types definitions for OL.

// Will be required in the future.
use ssz as _;

mod account;
mod epochal;
mod global;
mod ledger;
mod snark_account;
mod toplevel;

pub use account::*;
pub use epochal::*;
pub use global::*;
pub use ledger::*;
pub use snark_account::*;
pub use toplevel::*;
