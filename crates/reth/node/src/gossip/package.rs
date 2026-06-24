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

/// Size of the [`u32`] in bytes.
const U32_SIZE: usize = mem::size_of::<u32>();

/// Size of the fee config in bytes.
const FEE_CONFIG_SIZE: usize = SEQ_NO_SIZE + U32_SIZE + SEQ_NO_SIZE;

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

    /// Current static fee-model constants.
    fee_config: StaticFeeModelConfig,
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
    pub fn new(header: Header, seq_no: u64, fee_config: StaticFeeModelConfig) -> Self {
        Self {
            header,
            seq_no,
            fee_config,
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

    /// Gets the fee-model constants of the [`AlpenGossipMessage`].
    pub fn fee_config(&self) -> StaticFeeModelConfig {
        self.fee_config
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
        encode_fee_config(&self.fee_config, &mut buf);
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
        let fee_config = decode_fee_config(buf)?;

        Ok(Self {
            header,
            seq_no,
            fee_config,
        })
    }
}

fn encode_fee_config(fee_config: &StaticFeeModelConfig, buf: &mut BytesMut) {
    buf.put_u64(fee_config.prover_fee_per_gas_wei());
    buf.put_u32(fee_config.da_overhead_multiplier_bps());
    buf.put_u64(fee_config.ol_overhead_wei());
}

fn decode_fee_config(buf: &mut &[u8]) -> Result<StaticFeeModelConfig> {
    ensure!(
        buf.remaining() >= FEE_CONFIG_SIZE,
        "buffer too short for fee config: need {FEE_CONFIG_SIZE} bytes, have {}",
        buf.remaining()
    );

    Ok(StaticFeeModelConfig::new(
        buf.get_u64(),
        buf.get_u32(),
        buf.get_u64(),
    ))
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
    use strata_identifiers::test_utils::{buf32_strategy, buf64_strategy};

    use super::*;

    /// Strategy for generating arbitrary headers.
    /// For now, we use a default header since Header doesn't implement Arbitrary.
    /// In the future, this could be extended to generate headers with various fields.
    fn header_strategy() -> impl Strategy<Value = Header> {
        Just(Header::default())
    }

    fn fee_config_strategy() -> impl Strategy<Value = StaticFeeModelConfig> {
        (any::<u64>(), any::<u32>(), any::<u64>()).prop_map(
            |(prover_fee_per_gas_wei, da_overhead_multiplier_bps, ol_overhead_wei)| {
                StaticFeeModelConfig::new(
                    prover_fee_per_gas_wei,
                    da_overhead_multiplier_bps,
                    ol_overhead_wei,
                )
            },
        )
    }

    fn test_fee_config() -> StaticFeeModelConfig {
        StaticFeeModelConfig::new(15, 10_000, 0)
    }

    proptest! {
        #[test]
        fn test_message_encode_decode_roundtrip(
            header in header_strategy(),
            seq_no in any::<u64>(),
            fee_config in fee_config_strategy()
        ) {
            let original = AlpenGossipMessage::new(header, seq_no, fee_config);
            let encoded = original.encode();
            let decoded = AlpenGossipMessage::try_decode(&mut &encoded[..])
                .expect("decode should succeed");
            prop_assert_eq!(original, decoded);
        }

        #[test]
        fn test_message_encode_deterministic(
            header in header_strategy(),
            seq_no in any::<u64>(),
            fee_config in fee_config_strategy()
        ) {
            let msg = AlpenGossipMessage::new(header, seq_no, fee_config);
            let encoded1 = msg.encode();
            let encoded2 = msg.encode();
            prop_assert_eq!(encoded1, encoded2, "encoding should be deterministic");
        }

        #[test]
        fn test_message_getters(
            header in header_strategy(),
            seq_no in any::<u64>(),
            fee_config in fee_config_strategy()
        ) {
            let msg = AlpenGossipMessage::new(header.clone(), seq_no, fee_config);
            prop_assert_eq!(msg.header(), &header);
            prop_assert_eq!(msg.seq_no(), seq_no);
            prop_assert_eq!(msg.fee_config(), fee_config);
        }

        #[test]
        fn test_message_different_seq_no_different_encoding(
            header in header_strategy(),
            seq_no1 in any::<u64>(),
            seq_no2 in any::<u64>(),
            fee_config in fee_config_strategy()
        ) {
            prop_assume!(seq_no1 != seq_no2);
            let msg1 = AlpenGossipMessage::new(header.clone(), seq_no1, fee_config);
            let msg2 = AlpenGossipMessage::new(header, seq_no2, fee_config);
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

    #[test]
    fn test_message_try_decode_truncated_fee_config() {
        let mut encoded = AlpenGossipMessage::new(Header::default(), 1, test_fee_config()).encode();
        encoded.truncate(encoded.len() - 1);

        let result = AlpenGossipMessage::try_decode(&mut &encoded[..]);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("buffer too short for fee config"));
    }

    proptest! {
        #[test]
        fn test_package_encode_decode_roundtrip(
            header in header_strategy(),
            seq_no in any::<u64>(),
            fee_config in fee_config_strategy(),
            public_key in buf32_strategy(),
            signature in buf64_strategy()
        ) {
            let message = AlpenGossipMessage::new(header, seq_no, fee_config);
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
            fee_config in fee_config_strategy(),
            public_key in buf32_strategy(),
            signature in buf64_strategy()
        ) {
            let message = AlpenGossipMessage::new(header, seq_no, fee_config);
            let pkg = AlpenGossipPackage::new(message.clone(), public_key, signature);
            prop_assert_eq!(pkg.message(), &message);
            prop_assert_eq!(pkg.public_key(), &public_key);
            prop_assert_eq!(pkg.signature(), &signature);
        }

        #[test]
        fn test_package_encode_deterministic(
            header in header_strategy(),
            seq_no in any::<u64>(),
            fee_config in fee_config_strategy(),
            public_key in buf32_strategy(),
            signature in buf64_strategy()
        ) {
            let message = AlpenGossipMessage::new(header, seq_no, fee_config);
            let pkg = AlpenGossipPackage::new(message, public_key, signature);
            let encoded1 = pkg.encode();
            let encoded2 = pkg.encode();
            prop_assert_eq!(encoded1, encoded2, "encoding should be deterministic");
        }
    }

    #[test]
    fn test_signature_covers_fee_config() {
        let public_key = "0x1b84c5567b126440995d3ed5aaba0565d71e1834604819ff9c17f5e9d5dd078f"
            .parse::<Buf32>()
            .expect("public key should parse");
        let private_key = "0x0101010101010101010101010101010101010101010101010101010101010101"
            .parse::<Buf32>()
            .expect("private key should parse");

        let package = AlpenGossipMessage::new(Header::default(), 1, test_fee_config())
            .into_package(public_key, private_key);
        assert!(package.validate_signature());

        let altered_message = AlpenGossipMessage::new(
            package.message().header().clone(),
            package.message().seq_no(),
            StaticFeeModelConfig::new(16, 10_000, 0),
        );
        let altered_package =
            AlpenGossipPackage::new(altered_message, *package.public_key(), *package.signature());

        assert!(!altered_package.validate_signature());
    }
}
