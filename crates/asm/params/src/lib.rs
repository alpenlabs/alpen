//! Configuration parameters for the Anchor State Machine (ASM).
//!
//! Provides [`AsmParams`], which bundles the L1 magic bytes, genesis L1 view,
//! and per-subprotocol configuration (admin, bridge, checkpoint) needed to
//! initialize and run an ASM instance.

mod params;
mod subprotocols;

#[cfg(test)]
use proptest as _;

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

pub use params::AsmParams;
pub use ssz_generated::ssz::admin::{
    AdministrationInitConfigRef, CompressedPublicKeyBytes, RoleRef, ThresholdConfig,
    ThresholdConfigRef, ThresholdConfigUpdate, ThresholdConfigUpdateRef,
};
pub use subprotocols::{
    AdministrationInitConfig, BridgeV1InitConfig, CheckpointInitConfig, Role, SubprotocolInstance,
};
