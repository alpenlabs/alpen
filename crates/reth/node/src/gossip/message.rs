//! Gossip message types.
//!
//! Right now, we only support [`AlpenGossipMessage`].

use std::mem;

use alloy_primitives::bytes::{Buf, BufMut, BytesMut};
use alloy_rlp::{Decodable, Encodable};
use eyre::{ensure, eyre, Result};
use reth_primitives::Header;
use strata_primitives::{buf::Buf64, Buf32};

/// Size of the signature in bytes.
const SIGNATURE_SIZE: usize = Buf64::LEN;

/// Size of the sequence number in bytes.
const SEQ_NO_SIZE: usize = mem::size_of::<u64>();

/// Gossip message types.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlpenGossipMessage {
    /// Block [`Header`].
    header: Header,

    /// Sequence number.
    seq_no: u64,

    /// Sequencer's signature.
    signature: Buf64,
}

impl AlpenGossipMessage {
    /// Creates a new [`AlpenGossipMessage`].
    pub fn new(header: Header, seq_no: u64, signature: Buf64) -> Self {
        Self {
            header,
            seq_no,
            signature,
        }
    }

    /// Gets the header of the [`AlpenGossipMessage`].
    pub fn header(&self) -> &Header {
        &self.header
    }

    /// Gets the sequence number of the [`AlpenGossipMessage`].
    pub fn seq_no(&self) -> u64 {
        self.seq_no
    }

    /// Gets the signature of the [`AlpenGossipMessage`].
    pub fn signature(&self) -> &Buf64 {
        &self.signature
    }

    /// Validates the signature of the [`AlpenGossipMessage`] given a public key.
    pub fn validate_signature(&self, _public_key: &Buf32) -> bool {
        unimplemented!("Signature validation is not implemented yet");
    }

    /// Encodes a [`AlpenGossipMessage`] into bytes.
    pub(crate) fn encode(&self) -> BytesMut {
        let mut buf = BytesMut::new();
        self.header.encode(&mut buf);
        buf.put_u64(self.seq_no);
        buf.put_slice(self.signature.as_slice());
        buf
    }

    /// Decodes a [`AlpenGossipMessage`] from bytes with detailed error reporting.
    pub(crate) fn try_decode(buf: &mut &[u8]) -> Result<Self> {
        ensure!(!buf.is_empty(), "buffer is empty");

        // Decode the RLP-encoded header
        let header = Header::decode(buf).map_err(|e| eyre!("failed to decode RLP header: {e}"))?;

        // Check we have enough bytes for seq_no (u64 = 8 bytes)
        ensure!(
            buf.remaining() >= SEQ_NO_SIZE,
            "buffer too short for sequence number: need {SEQ_NO_SIZE} bytes, have {}",
            buf.remaining()
        );
        let seq_no = buf.get_u64();

        // Check we have enough bytes for signature (64 bytes)
        ensure!(
            buf.remaining() >= SIGNATURE_SIZE,
            "buffer too short for signature: need {SIGNATURE_SIZE} bytes, have {}",
            buf.remaining()
        );

        let signature_bytes: [u8; SIGNATURE_SIZE] = buf[..SIGNATURE_SIZE]
            .try_into()
            .expect("checked length above");
        buf.advance(SIGNATURE_SIZE);

        let signature = Buf64::from(signature_bytes);

        Ok(Self {
            header,
            seq_no,
            signature,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_header() -> Header {
        Header::default()
    }

    fn test_signature() -> Buf64 {
        Buf64::from([0xab; 64])
    }

    #[test]
    fn test_message_new() {
        let header = test_header();
        let seq_no = 42u64;
        let signature = test_signature();

        let msg = AlpenGossipMessage::new(header.clone(), seq_no, signature);

        assert_eq!(msg.header(), &header);
        assert_eq!(msg.seq_no(), seq_no);
        assert_eq!(msg.signature(), &signature);
    }

    #[test]
    fn test_message_encode_decode_roundtrip() {
        let header = test_header();
        let seq_no = 123u64;
        let signature = Buf64::from([0xcd; 64]);

        let original = AlpenGossipMessage::new(header, seq_no, signature);
        let encoded = original.encode();

        let decoded =
            AlpenGossipMessage::try_decode(&mut &encoded[..]).expect("decode should succeed");

        assert_eq!(original, decoded);
    }

    #[test]
    fn test_message_try_decode_empty_buffer() {
        let empty: &[u8] = &[];
        let result = AlpenGossipMessage::try_decode(&mut &empty[..]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("buffer is empty"));
    }

    #[test]
    fn test_message_decode_empty_buffer_returns_none() {
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
            test_header().encode(&mut buf);
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

    #[test]
    fn test_message_try_decode_truncated_signature() {
        let msg = AlpenGossipMessage::new(test_header(), 1, test_signature());
        let mut encoded = msg.encode();

        // Truncate to remove part of the signature (remove last 10 bytes)
        encoded.truncate(encoded.len() - 10);

        let result = AlpenGossipMessage::try_decode(&mut &encoded[..]);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("buffer too short for signature"));
    }

    #[test]
    fn test_message_getters() {
        let header = test_header();
        let seq_no = 999u64;
        let signature = Buf64::from([0x12; 64]);

        let msg = AlpenGossipMessage::new(header.clone(), seq_no, signature);

        assert_eq!(msg.header(), &header);
        assert_eq!(msg.seq_no(), 999);
        assert_eq!(msg.signature(), &Buf64::from([0x12; 64]));
    }

    #[test]
    fn test_message_encode_deterministic() {
        let header = test_header();
        let seq_no = 42u64;
        let signature = Buf64::from([0xaa; 64]);

        let msg = AlpenGossipMessage::new(header, seq_no, signature);

        let encoded1 = msg.encode();
        let encoded2 = msg.encode();

        assert_eq!(encoded1, encoded2, "encoding should be deterministic");
    }

    #[test]
    fn test_message_different_seq_no_different_encoding() {
        let header = test_header();
        let signature = Buf64::from([0xbb; 64]);

        let msg1 = AlpenGossipMessage::new(header.clone(), 1, signature);
        let msg2 = AlpenGossipMessage::new(header, 2, signature);

        assert_ne!(msg1.encode(), msg2.encode());
    }

    #[test]
    fn test_message_different_signature_different_encoding() {
        let header = test_header();
        let seq_no = 1u64;

        let msg1 = AlpenGossipMessage::new(header.clone(), seq_no, Buf64::from([0xaa; 64]));
        let msg2 = AlpenGossipMessage::new(header, seq_no, Buf64::from([0xbb; 64]));

        assert_ne!(msg1.encode(), msg2.encode());
    }

    #[test]
    fn test_message_roundtrip_with_various_seq_numbers() {
        for seq_no in [0u64, 1, 100, u64::MAX / 2, u64::MAX] {
            let msg = AlpenGossipMessage::new(test_header(), seq_no, test_signature());
            let encoded = msg.encode();
            let decoded =
                AlpenGossipMessage::try_decode(&mut &encoded[..]).expect("decode should succeed");
            assert_eq!(msg, decoded, "roundtrip failed for seq_no={seq_no}");
        }
    }
}
