use serde::{Deserialize, Serialize};

mod admin;
mod bridge;
mod checkpoint;

pub use admin::{AdministrationSubprotoParams, Role};
pub use bridge::BridgeV1Config;
pub use checkpoint::CheckpointConfig;

/// A configured subprotocol that can be registered in [`AsmParams`](crate::AsmParams).
///
/// Each variant carries the configuration for a single ASM subprotocol.
/// The list of instances stored in `AsmParams` determines which subprotocols
/// are active for a given network.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubprotocolInstance {
    /// Administration subprotocol for system upgrades.
    Admin(AdministrationSubprotoParams),

    /// Bridge V1 subprotocol for deposit/withdrawal management.
    Bridge(BridgeV1Config),

    /// Checkpoint subprotocol for OL checkpoint verification.
    Checkpoint(CheckpointConfig),
}
