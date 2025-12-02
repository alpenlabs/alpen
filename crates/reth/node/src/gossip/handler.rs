//! Handler for the custom RLPx subprotocol.
use std::net::SocketAddr;

use reth_network::protocol::ProtocolHandler;
use reth_network_api::{Direction, PeerId};
use tokio::sync::mpsc;

use crate::{
    gossip::connection::{AlpenGossipCommand, AlpenGossipConnectionHandler},
    AlpenGossipPackage,
};

/// Events emitted by the Alpen gossip protocol.
#[derive(Debug)]
#[expect(clippy::large_enum_variant, reason = "I don't want to box the thing")]
pub enum AlpenGossipEvent {
    /// New connection was established.
    Established {
        /// Peer that we established connection from/to.
        peer_id: PeerId,

        /// Direction of the connection.
        direction: Direction,

        /// Sender channel to the connection.
        to_connection: mpsc::UnboundedSender<AlpenGossipCommand>,
    },

    /// Connection was closed.
    Closed {
        /// Peer that we closed connection.
        peer_id: PeerId,
    },

    /// New package was received from a peer.
    Package {
        /// Peer that we received the message from.
        peer_id: PeerId,

        /// Received [`AlpenGossipPackage`].
        package: AlpenGossipPackage,
    },
}

/// State of the protocol.
#[derive(Clone, Debug)]
pub struct AlpenGossipState {
    /// Channel for sending events to the node.
    pub(crate) events: mpsc::UnboundedSender<AlpenGossipEvent>,
}

impl AlpenGossipState {
    /// Creates a new [`AlpenGossipState`].
    pub fn new(events: mpsc::UnboundedSender<AlpenGossipEvent>) -> Self {
        Self { events }
    }

    /// Gets the channel for sending events to the node.
    pub fn events(&self) -> &mpsc::UnboundedSender<AlpenGossipEvent> {
        &self.events
    }
}

/// The protocol handler for Alpen gossip.
#[derive(Debug)]
pub struct AlpenGossipProtocolHandler {
    /// State of the Alpen gossip protocol.
    state: AlpenGossipState,
}

impl AlpenGossipProtocolHandler {
    /// Creates a new [`AlpenGossipProtocolHandler`] with the given state.
    pub fn new(state: AlpenGossipState) -> Self {
        Self { state }
    }

    /// Gets the current state of the Alpen gossip protocol.
    pub fn state(&self) -> &AlpenGossipState {
        &self.state
    }
}

impl ProtocolHandler for AlpenGossipProtocolHandler {
    type ConnectionHandler = AlpenGossipConnectionHandler;

    fn on_incoming(&self, _socket_addr: SocketAddr) -> Option<Self::ConnectionHandler> {
        Some(AlpenGossipConnectionHandler {
            state: self.state.clone(),
        })
    }

    fn on_outgoing(
        &self,
        _socket_addr: SocketAddr,
        _peer_id: PeerId,
    ) -> Option<Self::ConnectionHandler> {
        Some(AlpenGossipConnectionHandler {
            state: self.state.clone(),
        })
    }
}
