//! Gossip package and message types.
//!
//! Right now, we only support [`AlpenGossipPackage`] and [`AlpenGossipMessage`].

use std::mem;

use alloy_primitives::{
    bytes::{Buf, BufMut, BytesMut},
    eip191_hash_message,
};
use alloy_rlp::{Decodable, Encodable};
use eyre::{ensure, eyre, Result};
use reth_primitives::Header;
use strata_primitives::{
    buf::Buf64,
    crypto::{sign_schnorr_sig, verify_schnorr_sig},
    Buf32,
};

/// Size of the sequence number in bytes.
const SEQ_NO_SIZE: usize = mem::size_of::<u64>();

/// Message ID for gossip packages
/// This is prepended to each Message
const GOSSIP_PACKAGE_MSG_ID: u8 = 0x00;

/// Gossip message types.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlpenGossipMessage {
    /// Block [`Header`].
    header: Header,

    /// Sequence number.
    seq_no: u64,
}

/// Alpen Gossip Package that contains the message and the signature.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlpenGossipPackage {
    /// Alpen Gossip Message.
    message: AlpenGossipMessage,

    /// Sender's public key.
    public_key: Buf32,

    /// Sender's signature.
    signature: Buf64,
}

impl AlpenGossipMessage {
    /// Creates a new [`AlpenGossipMessage`].
    pub fn new(header: Header, seq_no: u64) -> Self {
        Self { header, seq_no }
    }

    /// Gets the header of the [`AlpenGossipMessage`].
    pub fn header(&self) -> &Header {
        &self.header
    }

    /// Gets the sequence number of the [`AlpenGossipMessage`].
    pub fn seq_no(&self) -> u64 {
        self.seq_no
    }

    /// Gets the hash of the [`AlpenGossipMessage`].
    ///
    /// The hash is computed using EIP-191 (Keccak-256) and then converted to a [`Buf32`].
    pub fn hash(&self) -> Buf32 {
        Buf32::from(eip191_hash_message(self.encode()).0)
    }

    /// Consumes the [`AlpenGossipMessage`] into a [`AlpenGossipPackage`] by signing the message
    /// with the `private_key` and given a `public_key`.
    ///
    /// The message is hashed using EIP-191 (Keccak-256) and then signed with the `private_key`.
    pub fn into_package(self, public_key: Buf32, private_key: Buf32) -> AlpenGossipPackage {
        let signature = sign_schnorr_sig(&self.hash(), &private_key);
        AlpenGossipPackage::new(self, public_key, signature)
    }

    /// Encodes a [`AlpenGossipMessage`] into bytes.
    pub(crate) fn encode(&self) -> BytesMut {
        let mut buf = BytesMut::new();
        self.header.encode(&mut buf);
        buf.put_u64(self.seq_no);
        buf
    }

    /// Decodes a [`AlpenGossipMessage`] from bytes with detailed error reporting.
    pub(crate) fn try_decode(buf: &mut &[u8]) -> Result<Self> {
        ensure!(!buf.is_empty(), "buffer is empty");

        // Decode the RLP-encoded header
        // Header::decode already advances the buffer during RLP decoding
        let header = Header::decode(buf).map_err(|e| eyre!("failed to decode RLP header: {e}"))?;

        // Check we have enough bytes for seq_no (u64 = 8 bytes)
        ensure!(
            buf.remaining() >= SEQ_NO_SIZE,
            "buffer too short for sequence number: need {SEQ_NO_SIZE} bytes, have {}",
            buf.remaining()
        );

        // Decode the sequence number
        let seq_no = buf.get_u64();

        Ok(Self { header, seq_no })
    }
}

impl AlpenGossipPackage {
    /// Creates a new [`AlpenGossipPackage`].
    pub(crate) fn new(message: AlpenGossipMessage, public_key: Buf32, signature: Buf64) -> Self {
        Self {
            message,
            public_key,
            signature,
        }
    }

    /// Gets the message of the [`AlpenGossipPackage`].
    pub fn message(&self) -> &AlpenGossipMessage {
        &self.message
    }

    /// Gets the public key of the [`AlpenGossipPackage`].
    pub fn public_key(&self) -> &Buf32 {
        &self.public_key
    }

    /// Gets the signature of the [`AlpenGossipPackage`].
    pub fn signature(&self) -> &Buf64 {
        &self.signature
    }

    /// Validates the signature of the [`AlpenGossipPackage`].
    pub fn validate_signature(&self) -> bool {
        let message = self.message.encode().to_vec();
        let hash = Buf32::from(eip191_hash_message(message).0);
        let signature = self.signature();
        let public_key = self.public_key();
        verify_schnorr_sig(signature, &hash, public_key)
    }

    /// Encodes a [`AlpenGossipPackage`] into bytes for wire transmission.
    ///
    /// The format is: `msg_id || message || public_key || signature`
    /// where `msg_id` is a single byte that identifies the message type within
    /// the alpen_gossip subprotocol. The RLPx multiplexer will add an offset
    /// to this byte to produce the final wire message ID.
    pub(crate) fn encode(&self) -> BytesMut {
        let mut buf = BytesMut::new();
        // Prepend message ID for the RLPx multiplexer
        buf.put_u8(GOSSIP_PACKAGE_MSG_ID);
        let message = self.message.encode();
        buf.put_slice(&message);
        buf.put_slice(&self.public_key.0);
        buf.put_slice(&self.signature.0);
        buf
    }

    /// Decodes a [`AlpenGossipPackage`] from bytes with detailed error reporting.
    ///
    /// The format is: `msg_id || message || public_key || signature`
    /// where `msg_id` has been normalized by the RLPx multiplexer (offset subtracted).
    pub(crate) fn try_decode(buf: &mut &[u8]) -> Result<Self> {
        ensure!(!buf.is_empty(), "buffer is empty");

        // Read and verify message ID
        let msg_id = buf.get_u8();
        ensure!(
            msg_id == GOSSIP_PACKAGE_MSG_ID,
            "unexpected gossip message type: expected {GOSSIP_PACKAGE_MSG_ID}, got {msg_id}"
        );

        let message = AlpenGossipMessage::try_decode(buf)?;

        // Check we have enough bytes for public key and signature
        ensure!(
            buf.remaining() >= Buf32::LEN + Buf64::LEN,
            "buffer too short for public key and signature: need {} bytes, have {}",
            Buf32::LEN + Buf64::LEN,
            buf.remaining()
        );

        // Extract public key (32 bytes)
        let mut public_key_bytes = [0u8; Buf32::LEN];
        buf.copy_to_slice(&mut public_key_bytes);
        let public_key = Buf32::from(public_key_bytes);

        // Extract signature (64 bytes)
        let mut signature_bytes = [0u8; Buf64::LEN];
        buf.copy_to_slice(&mut signature_bytes);
        let signature = Buf64::from(signature_bytes);

        Ok(Self {
            message,
            public_key,
            signature,
        })
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    /// Strategy for generating arbitrary headers.
    /// For now, we use a default header since Header doesn't implement Arbitrary.
    /// In the future, this could be extended to generate headers with various fields.
    fn header_strategy() -> impl Strategy<Value = Header> {
        Just(Header::default())
    }

    /// Strategy for generating arbitrary Buf32 values.
    fn buf32_strategy() -> impl Strategy<Value = Buf32> {
        prop::array::uniform32(0u8..).prop_map(Buf32::from)
    }

    /// Strategy for generating arbitrary Buf64 values.
    fn buf64_strategy() -> impl Strategy<Value = Buf64> {
        prop::collection::vec(0u8.., 64).prop_map(|vec| {
            let mut arr = [0u8; 64];
            arr.copy_from_slice(&vec);
            Buf64::from(arr)
        })
    }

    proptest! {
        #[test]
        fn test_message_encode_decode_roundtrip(
            header in header_strategy(),
            seq_no in any::<u64>()
        ) {
            let original = AlpenGossipMessage::new(header, seq_no);
            let encoded = original.encode();
            let decoded = AlpenGossipMessage::try_decode(&mut &encoded[..])
                .expect("decode should succeed");
            prop_assert_eq!(original, decoded);
        }

        #[test]
        fn test_message_encode_deterministic(
            header in header_strategy(),
            seq_no in any::<u64>()
        ) {
            let msg = AlpenGossipMessage::new(header, seq_no);
            let encoded1 = msg.encode();
            let encoded2 = msg.encode();
            prop_assert_eq!(encoded1, encoded2, "encoding should be deterministic");
        }

        #[test]
        fn test_message_getters(
            header in header_strategy(),
            seq_no in any::<u64>()
        ) {
            let msg = AlpenGossipMessage::new(header.clone(), seq_no);
            prop_assert_eq!(msg.header(), &header);
            prop_assert_eq!(msg.seq_no(), seq_no);
        }

        #[test]
        fn test_message_different_seq_no_different_encoding(
            header in header_strategy(),
            seq_no1 in any::<u64>(),
            seq_no2 in any::<u64>()
        ) {
            prop_assume!(seq_no1 != seq_no2);
            let msg1 = AlpenGossipMessage::new(header.clone(), seq_no1);
            let msg2 = AlpenGossipMessage::new(header, seq_no2);
            prop_assert_ne!(msg1.encode(), msg2.encode());
        }
    }

    #[test]
    fn test_message_try_decode_empty_buffer() {
        let empty: &[u8] = &[];
        let result = AlpenGossipMessage::try_decode(&mut &empty[..]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("buffer is empty"));
    }

    #[test]
    fn test_message_try_decode_invalid_header() {
        // Invalid RLP data
        let invalid: &[u8] = &[0xff, 0xff, 0xff];
        let result = AlpenGossipMessage::try_decode(&mut &invalid[..]);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("failed to decode RLP header"));
    }

    #[test]
    fn test_message_try_decode_truncated_seq_no() {
        // Get only the header portion (truncate before seq_no)
        // Header is RLP encoded, so we need to find where it ends
        let header_only = {
            let mut buf = BytesMut::new();
            Header::default().encode(&mut buf);
            buf
        };

        // Add only partial seq_no bytes (less than 8)
        let mut truncated = header_only.to_vec();
        truncated.extend_from_slice(&[0u8; 4]); // Only 4 bytes instead of 8

        let result = AlpenGossipMessage::try_decode(&mut &truncated[..]);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("buffer too short for sequence number"));
    }

    proptest! {
        #[test]
        fn test_package_encode_decode_roundtrip(
            header in header_strategy(),
            seq_no in any::<u64>(),
            public_key in buf32_strategy(),
            signature in buf64_strategy()
        ) {
            let message = AlpenGossipMessage::new(header, seq_no);
            let original = AlpenGossipPackage::new(message, public_key, signature);
            let encoded = original.encode();
            let decoded = AlpenGossipPackage::try_decode(&mut &encoded[..])
                .expect("decode should succeed");
            prop_assert_eq!(original, decoded);
        }

        #[test]
        fn test_package_getters(
            header in header_strategy(),
            seq_no in any::<u64>(),
            public_key in buf32_strategy(),
            signature in buf64_strategy()
        ) {
            let message = AlpenGossipMessage::new(header, seq_no);
            let pkg = AlpenGossipPackage::new(message.clone(), public_key, signature);
            prop_assert_eq!(pkg.message(), &message);
            prop_assert_eq!(pkg.public_key(), &public_key);
            prop_assert_eq!(pkg.signature(), &signature);
        }

        #[test]
        fn test_package_encode_deterministic(
            header in header_strategy(),
            seq_no in any::<u64>(),
            public_key in buf32_strategy(),
            signature in buf64_strategy()
        ) {
            let message = AlpenGossipMessage::new(header, seq_no);
            let pkg = AlpenGossipPackage::new(message, public_key, signature);
            let encoded1 = pkg.encode();
            let encoded2 = pkg.encode();
            prop_assert_eq!(encoded1, encoded2, "encoding should be deterministic");
        }
    }
}
