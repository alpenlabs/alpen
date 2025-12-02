//! Connection handler for the custom RLPx subprotocol.
use std::{
    pin::Pin,
    task::{ready, Context, Poll},
};

use alloy_primitives::bytes::BytesMut;
use futures::{Stream, StreamExt};
use reth_eth_wire::{
    capability::SharedCapabilities, multiplex::ProtocolConnection, protocol::Protocol,
};
use reth_network::protocol::{ConnectionHandler, OnNotSupported};
use reth_network_api::{Direction, PeerId};
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tracing::{error, warn};

use crate::{
    gossip::{
        handler::{AlpenGossipEvent, AlpenGossipState},
        protocol::alpen_gossip_protocol,
    },
    AlpenGossipPackage,
};

/// Command to send to the connection.
#[derive(Debug)]
pub enum AlpenGossipCommand {
    /// Send a package to the peer.
    SendPackage(AlpenGossipPackage),
}

/// Connection handler for the Alpen gossip protocol.
#[derive(Debug)]
pub struct AlpenGossipConnectionHandler {
    /// Alpen gossip state.
    pub(crate) state: AlpenGossipState,
}

impl AlpenGossipConnectionHandler {
    /// Creates a new [`AlpenGossipConnectionHandler`] with the given state.
    pub fn new(state: AlpenGossipState) -> Self {
        Self { state }
    }

    /// Gets the current state of the Alpen gossip protocol.
    pub fn state(&self) -> &AlpenGossipState {
        &self.state
    }
}

impl ConnectionHandler for AlpenGossipConnectionHandler {
    type Connection = AlpenGossipConnection;

    fn protocol(&self) -> Protocol {
        alpen_gossip_protocol()
    }

    fn on_unsupported_by_peer(
        self,
        _supported: &SharedCapabilities,
        direction: Direction,
        peer_id: PeerId,
    ) -> OnNotSupported {
        warn!(
            target: "alpen-gossip",
            %peer_id,
            ?direction,
            "Peer does not support alpen_gossip protocol, disconnecting"
        );
        OnNotSupported::Disconnect
    }

    fn into_connection(
        self,
        direction: Direction,
        peer_id: PeerId,
        conn: ProtocolConnection,
    ) -> Self::Connection {
        let (tx, rx) = mpsc::unbounded_channel();
        self.state
            .events
            .send(AlpenGossipEvent::Established {
                peer_id,
                direction,
                to_connection: tx,
            })
            .ok();

        AlpenGossipConnection {
            conn,
            commands: UnboundedReceiverStream::new(rx),
            peer_id,
            events: self.state.events.clone(),
        }
    }
}

/// Connection for the Alpen gossip protocol.
#[derive(Debug)]
pub struct AlpenGossipConnection {
    /// Protocol connection.
    conn: ProtocolConnection,

    /// Command stream.
    commands: UnboundedReceiverStream<AlpenGossipCommand>,

    /// Peer id.
    peer_id: PeerId,

    /// Event sender.
    events: mpsc::UnboundedSender<AlpenGossipEvent>,
}

impl Drop for AlpenGossipConnection {
    fn drop(&mut self) {
        self.events
            .send(AlpenGossipEvent::Closed {
                peer_id: self.peer_id,
            })
            .ok();
    }
}

impl Stream for AlpenGossipConnection {
    type Item = BytesMut;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        loop {
            // Poll for outgoing messages
            if let Poll::Ready(Some(cmd)) = this.commands.poll_next_unpin(cx) {
                return match cmd {
                    AlpenGossipCommand::SendPackage(package) => Poll::Ready(Some(package.encode())),
                };
            }

            // Poll for incoming messages
            let Some(msg) = ready!(this.conn.poll_next_unpin(cx)) else {
                return Poll::Ready(None);
            };

            let pkg = match AlpenGossipPackage::try_decode(&mut &msg[..]) {
                Ok(msg) => msg,
                Err(e) => {
                    error!(
                        target: "alpen-gossip",
                        peer_id = %this.peer_id,
                        err = ?e,
                        message_len = %msg.len(),
                        "Failed to decode gossip package from peer, disconnecting"
                    );
                    return Poll::Ready(None);
                }
            };

            this.events
                .send(AlpenGossipEvent::Package {
                    peer_id: this.peer_id,
                    package: pkg,
                })
                .ok();
        }
    }
}
