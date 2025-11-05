//! The crate provides common types and traits for building blocks for defining
//! and interacting with subprotocols in an ASM (Anchor State Machine) framework.

mod aux;
mod error;
mod log;
mod manifest;
mod msg;
mod spec;
mod state;
mod subprotocol;
mod tx;

pub use aux::*;
pub use error::*;
pub use log::*;
pub use manifest::*;
pub use msg::*;
pub use spec::*;
pub use state::*;
// Re-export MMR types from acct-types for convenience
pub use strata_acct_types::{CompactMmr64, Mmr64};
pub use subprotocol::*;
use tracing as _;
pub use tx::*;

// Re-export the logging module
pub mod logging;
