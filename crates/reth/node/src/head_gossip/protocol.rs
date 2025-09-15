//! RLPx subprotocol for gossiping block head hashes.

use alloy_primitives::bytes::{Buf, BufMut, BytesMut};
use alloy_rlp::{Decodable, Encodable};
use reth_eth_wire::{protocol::Protocol, Capability};
use reth_primitives::Header;

/// Head gossip protocol name.
const PROTOCOL_NAME: &str = "head_gossip";

/// Head gossip protocol version.
const PROTOCOL_VERSION: usize = 1;

/// [`Capability`] for the `head_gossip` protocol with version `1`.
pub(crate) const HEAD_GOSSIP_CAPABILITY: Capability =
    Capability::new_static(PROTOCOL_NAME, PROTOCOL_VERSION);

/// [`Protocol`] for the `head_gossip` protocol.
pub(crate) fn head_gossip_protocol() -> Protocol {
    // total packets = 2
    Protocol::new(HEAD_GOSSIP_CAPABILITY, 2)
}

/// Head gossip protocol message IDs.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HeadGossipMessageId {
    /// ID when sending/receiving a single [`Header`].
    HeadHash = 0x00,

    /// ID when sending/receiving multiple [`Header`]s.
    HeadHashes = 0x01,
}

/// Head gossip protocol message kinds.
#[expect(clippy::large_enum_variant, reason = "I don't want to box the thing")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum HeadGossipMessageKind {
    /// Single [`Header`]
    HeadHash(Header),

    /// Multiple [`Header`]s.
    HeadHashes(Vec<Header>),
}

/// Head gossip protocol messages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HeadGossipMessage {
    /// Message type.
    pub message_type: HeadGossipMessageId,

    /// Underlying message.
    pub message: HeadGossipMessageKind,
}

impl HeadGossipMessage {
    /// Creates a new [`HeadGossipMessage`] with a single [`Header`].
    pub(crate) fn new_head_hash(header: Header) -> Self {
        Self {
            message_type: HeadGossipMessageId::HeadHash,
            message: HeadGossipMessageKind::HeadHash(header),
        }
    }

    /// Creates a new [`HeadGossipMessage`] with multiple [`Header`]s.
    pub(crate) fn new_head_hashes(headers: Vec<Header>) -> Self {
        Self {
            message_type: HeadGossipMessageId::HeadHashes,
            message: HeadGossipMessageKind::HeadHashes(headers),
        }
    }

    /// Encodes a [`HeadGossipMessage`] into bytes.
    pub(crate) fn encoded(&self) -> BytesMut {
        let mut buf = BytesMut::new();
        buf.put_u8(self.message_type as u8);
        match &self.message {
            HeadGossipMessageKind::HeadHash(header) => {
                header.encode(&mut buf);
            }
            HeadGossipMessageKind::HeadHashes(headers) => {
                headers.encode(&mut buf);
            }
        }
        buf
    }

    /// Decodes a [`HeadGossipMessage`] into bytes.
    pub(crate) fn decode_message(buf: &mut &[u8]) -> Option<Self> {
        if buf.is_empty() {
            return None;
        }
        let id = buf[0];
        buf.advance(1);
        let message_type = match id {
            0x00 => HeadGossipMessageId::HeadHash,
            0x01 => HeadGossipMessageId::HeadHashes,
            _ => return None,
        };
        let message = match message_type {
            HeadGossipMessageId::HeadHash => {
                let header = Header::decode(buf).ok()?;
                HeadGossipMessageKind::HeadHash(header)
            }
            HeadGossipMessageId::HeadHashes => {
                let headers = Vec::<Header>::decode(buf).ok()?;
                HeadGossipMessageKind::HeadHashes(headers)
            }
        };

        Some(Self {
            message_type,
            message,
        })
    }
}
