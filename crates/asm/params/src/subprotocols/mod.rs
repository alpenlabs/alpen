use serde::{Deserialize, Serialize};

mod admin;
mod bridge;
mod checkpoint;

pub use admin::{AdministrationSubprotoParams, Role};
pub use bridge::BridgeV1Config;
pub use checkpoint::CheckpointConfig;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum SubprotocolInstance {
    Admin(AdministrationSubprotoParams),
    Bridge(BridgeV1Config),
    Checkpoint(CheckpointConfig),
}
