mod message;
mod util;

pub use message::{BridgeMessage, BridgeMsgId, Scope};
pub use util::{verify_bridge_msg_sig, MessageSigner, VerifyError};
