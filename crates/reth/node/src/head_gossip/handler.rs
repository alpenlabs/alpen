//! Handler for the custom RLPx subprotocol.
use std::net::SocketAddr;

use reth_network::protocol::ProtocolHandler;
use reth_network_api::{Direction, PeerId};
use reth_primitives::Header;
use tokio::sync::mpsc;

use crate::head_gossip::connection::{HeadGossipCommand, HeadGossipConnectionHandler};

/// Events emitted by the head gossip protocol.
#[derive(Debug)]
#[expect(clippy::large_enum_variant, reason = "I don't want to box the thing")]
pub enum HeadGossipEvent {
    /// New connection was established.
    Established {
        /// Peer that we established connection from/to.
        peer_id: PeerId,

        /// Direction of the connection.
        direction: Direction,

        /// Sender channel to the connection.
        to_connection: mpsc::UnboundedSender<HeadGossipCommand>,
    },

    /// Connection was closed.
    Closed {
        /// Peer that we closed connection.
        peer_id: PeerId,
    },

    /// New head hash was received from a peer.
    HeadHash {
        /// Peer that we received the new head hash.
        peer_id: PeerId,

        /// Received [`Header`].
        header: Header,
    },

    /// Multiple head hashes were received from a peer.
    HeadHashes {
        /// Peer that we received the new head hashes.
        peer_id: PeerId,

        /// Received [`Header`]s.
        headers: Vec<Header>,
    },
}

/// State of the protocol.
#[derive(Clone, Debug)]
pub struct HeadGossipState {
    /// Channel for sending events to the node.
    pub events: mpsc::UnboundedSender<HeadGossipEvent>,
}

/// The protocol handler for head gossip.
#[derive(Debug)]
pub struct HeadGossipProtocolHandler {
    /// State of the head gossip protocol.
    pub state: HeadGossipState,
}

impl ProtocolHandler for HeadGossipProtocolHandler {
    type ConnectionHandler = HeadGossipConnectionHandler;

    fn on_incoming(&self, _socket_addr: SocketAddr) -> Option<Self::ConnectionHandler> {
        Some(HeadGossipConnectionHandler {
            state: self.state.clone(),
        })
    }

    fn on_outgoing(
        &self,
        _socket_addr: SocketAddr,
        _peer_id: PeerId,
    ) -> Option<Self::ConnectionHandler> {
        Some(HeadGossipConnectionHandler {
            state: self.state.clone(),
        })
    }
}
