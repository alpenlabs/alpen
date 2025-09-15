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
use reth_primitives::Header;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;

use crate::head_gossip::{
    handler::{HeadGossipEvent, HeadGossipState},
    protocol::{head_gossip_protocol, HeadGossipMessage, HeadGossipMessageKind},
};

/// Command to send to the connection.
#[expect(clippy::large_enum_variant, reason = "I don't want to box the thing")]
#[derive(Debug)]
pub enum HeadGossipCommand {
    /// Send a head hash to the peer.
    SendHeadHash(Header),

    /// Send multiple head hashes to the peer.
    SendHeadHashes(Vec<Header>),
}

/// Connection handler for the head gossip protocol.
#[derive(Debug)]
pub struct HeadGossipConnectionHandler {
    /// Head gossip state.
    pub(crate) state: HeadGossipState,
}

impl ConnectionHandler for HeadGossipConnectionHandler {
    type Connection = HeadGossipConnection;

    fn protocol(&self) -> Protocol {
        head_gossip_protocol()
    }

    fn on_unsupported_by_peer(
        self,
        _supported: &SharedCapabilities,
        _direction: Direction,
        _peer_id: PeerId,
    ) -> OnNotSupported {
        // make it simple and keep it alive even if other peers do not support
        OnNotSupported::KeepAlive
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
            .send(HeadGossipEvent::Established {
                peer_id,
                direction,
                to_connection: tx,
            })
            .ok();

        HeadGossipConnection {
            conn,
            commands: UnboundedReceiverStream::new(rx),
            peer_id,
            events: self.state.events.clone(),
        }
    }
}

/// Connection for the head gossip protocol.
#[derive(Debug)]
pub struct HeadGossipConnection {
    /// Protocol connection.
    conn: ProtocolConnection,

    /// Command stream.
    commands: UnboundedReceiverStream<HeadGossipCommand>,

    /// Peer id.
    peer_id: PeerId,

    /// Event sender.
    events: mpsc::UnboundedSender<HeadGossipEvent>,
}

impl Drop for HeadGossipConnection {
    fn drop(&mut self) {
        self.events
            .send(HeadGossipEvent::Closed {
                peer_id: self.peer_id,
            })
            .ok();
    }
}

impl Stream for HeadGossipConnection {
    type Item = BytesMut;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        loop {
            // Poll for outgoing messages
            if let Poll::Ready(Some(cmd)) = this.commands.poll_next_unpin(cx) {
                let msg = match cmd {
                    HeadGossipCommand::SendHeadHash(header) => {
                        HeadGossipMessage::new_head_hash(header)
                    }
                    HeadGossipCommand::SendHeadHashes(headers) => {
                        HeadGossipMessage::new_head_hashes(headers)
                    }
                };
                return Poll::Ready(Some(msg.encoded()));
            }

            // Poll for incoming messages
            let Some(msg) = ready!(this.conn.poll_next_unpin(cx)) else {
                return Poll::Ready(None);
            };

            let Some(msg) = HeadGossipMessage::decode_message(&mut &msg[..]) else {
                // TODO(@storopoli): maybe disconnect on invalid message
                return Poll::Ready(None);
            };

            match msg.message {
                HeadGossipMessageKind::HeadHash(header) => {
                    this.events
                        .send(HeadGossipEvent::HeadHash {
                            peer_id: this.peer_id,
                            header,
                        })
                        .ok();
                }
                HeadGossipMessageKind::HeadHashes(headers) => {
                    this.events
                        .send(HeadGossipEvent::HeadHashes {
                            peer_id: this.peer_id,
                            headers,
                        })
                        .ok();
                }
            }
        }
    }
}
