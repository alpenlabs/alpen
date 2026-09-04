//! OL parameters.
//!
//! Provides JSON-serializable configuration for OL genesis state, including
//! genesis block header parameters, genesis account definitions, and the
//! initial L1 block commitment, plus runtime parameters that affect OL STF
//! execution.

mod account;
mod header;
mod params;

pub use account::GenesisSnarkAccountData;
pub use header::GenesisHeaderParams;
pub use params::{OLGenesisParams, OLParams, OLParamsBuilder, OLRuntimeParams};
pub use strata_bridge_params::BridgeParams;
