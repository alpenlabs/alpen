//! Configuration parameters for the Anchor State Machine (ASM).
//!
//! Provides [`AsmParams`], which bundles the L1 magic bytes, genesis L1 view,
//! and per-subprotocol configuration (admin, bridge, checkpoint) needed to
//! initialize and run an ASM instance.

mod params;
mod subprotocols;

pub use params::AsmParams;
pub use subprotocols::{
    AdministrationSubprotoParams, BridgeV1Config, CheckpointConfig, Role, SubprotocolInstance,
};
