//! RLPx subprotocol for gossiping block head hashes.

use reth_eth_wire::{protocol::Protocol, Capability};

/// Alpen gossip protocol name.
const PROTOCOL_NAME: &str = "alpen_gossip";

/// Alpen gossip protocol version.
const PROTOCOL_VERSION: usize = 1;

/// [`Capability`] for the `alpen_gossip` protocol with version `1`.
pub(crate) const ALPEN_GOSSIP_CAPABILITY: Capability =
    Capability::new_static(PROTOCOL_NAME, PROTOCOL_VERSION);

/// [`Protocol`] for the `alpen_gossip` protocol.
pub(crate) fn alpen_gossip_protocol() -> Protocol {
    // total packets = 2
    Protocol::new(ALPEN_GOSSIP_CAPABILITY, 2)
}
