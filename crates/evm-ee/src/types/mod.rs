//! EVM Execution Environment types.
//!
//! This module defines the types needed for EVM block execution within the
//! ExecutionEnvironment trait framework.

use strata_codec::{Codec, CodecError};

pub(crate) type Hash = [u8; 32];

// Module declarations
mod partial_state;
mod write_batch;
mod header;
mod block_body;
mod block;

// Re-export public types
pub use partial_state::EvmPartialState;
pub use write_batch::EvmWriteBatch;
pub use header::EvmHeader;
pub use block_body::EvmBlockBody;
pub use block::EvmBlock;

// Keep tests module
#[cfg(test)]
mod tests;

/// Helper function to encode an RLP-encodable item with length prefix.
///
/// This encodes the item using RLP, then writes a u32 length prefix followed by the RLP bytes.
fn encode_rlp_with_length<T: alloy_rlp::Encodable>(
    item: &T,
    enc: &mut impl strata_codec::Encoder,
) -> Result<(), CodecError> {
    let rlp_encoded = alloy_rlp::encode(item);
    (rlp_encoded.len() as u32).encode(enc)?;
    enc.write_buf(&rlp_encoded)?;
    Ok(())
}

/// Helper function to decode an RLP-decodable item with length prefix.
///
/// This reads a u32 length prefix, then reads that many bytes and decodes them using RLP.
fn decode_rlp_with_length<T: alloy_rlp::Decodable>(
    dec: &mut impl strata_codec::Decoder,
) -> Result<T, CodecError> {
    let len = u32::decode(dec)? as usize;
    let mut buf = vec![0u8; len];
    dec.read_buf(&mut buf)?;

    alloy_rlp::Decodable::decode(&mut &buf[..])
        .map_err(|_| CodecError::MalformedField("RLP decode failed"))
}

/// Helper function to encode raw bytes with length prefix.
///
/// This writes a u32 length prefix followed by the raw bytes.
fn encode_bytes_with_length(
    bytes: &[u8],
    enc: &mut impl strata_codec::Encoder,
) -> Result<(), CodecError> {
    (bytes.len() as u32).encode(enc)?;
    enc.write_buf(bytes)?;
    Ok(())
}

/// Helper function to decode raw bytes with length prefix.
///
/// This reads a u32 length prefix, then reads that many bytes and returns them as a Vec<u8>.
fn decode_bytes_with_length(dec: &mut impl strata_codec::Decoder) -> Result<Vec<u8>, CodecError> {
    let len = u32::decode(dec)? as usize;
    let mut bytes = vec![0u8; len];
    dec.read_buf(&mut bytes)?;
    Ok(bytes)
}
