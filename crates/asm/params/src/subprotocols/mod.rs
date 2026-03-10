use serde::{Deserialize, Serialize};

mod admin;
mod bridge;
mod checkpoint;

pub use admin::{AdministrationInitConfig, Role};
pub use bridge::BridgeV1InitConfig;
pub use checkpoint::CheckpointInitConfig;

/// A configured subprotocol that can be registered in [`AsmParams`](crate::AsmParams).
///
/// Each variant carries the configuration for a single ASM subprotocol.
/// The list of instances stored in `AsmParams` determines which subprotocols
/// are active for a given network.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubprotocolInstance {
    /// Administration subprotocol for system upgrades.
    Admin(AdministrationInitConfig),

    /// Bridge V1 subprotocol for deposit/withdrawal management.
    Bridge(BridgeV1InitConfig),

    /// Checkpoint subprotocol for OL checkpoint verification.
    Checkpoint(CheckpointInitConfig),
}
