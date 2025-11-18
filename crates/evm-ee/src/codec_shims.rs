//! Codec helper functions for encoding/decoding with length prefixes.
//!
//! This module provides utility functions for encoding and decoding data with length prefixes,
//! useful for variable-length fields in the EVM execution environment types.

use strata_codec::{Codec, CodecError, Varint};

/// Encodes an RLP-encodable item with a varint length prefix.
///
/// This encodes the item using RLP, then writes a varint length prefix followed by the RLP bytes.
/// Varints are more space-efficient for small lengths.
pub(crate) fn encode_rlp_with_length<T: alloy_rlp::Encodable>(
    item: &T,
    enc: &mut impl strata_codec::Encoder,
) -> Result<(), CodecError> {
    let rlp_encoded = alloy_rlp::encode(item);
    let len = Varint::new(rlp_encoded.len() as u32)
        .ok_or(CodecError::MalformedField("length too large for varint"))?;
    len.encode(enc)?;
    enc.write_buf(&rlp_encoded)?;
    Ok(())
}

/// Decodes an RLP-decodable item with a varint length prefix.
///
/// This reads a varint length prefix, then reads that many bytes and decodes them using RLP.
pub(crate) fn decode_rlp_with_length<T: alloy_rlp::Decodable>(
    dec: &mut impl strata_codec::Decoder,
) -> Result<T, CodecError> {
    let len_varint = Varint::decode(dec)?;
    let len = len_varint.inner() as usize;
    let mut buf = vec![0u8; len];
    dec.read_buf(&mut buf)?;

    alloy_rlp::Decodable::decode(&mut &buf[..])
        .map_err(|_| CodecError::MalformedField("RLP decode failed"))
}

/// Encodes raw bytes with a varint length prefix.
///
/// This writes a varint length prefix followed by the raw bytes.
/// Varints are more space-efficient for small lengths.
pub(crate) fn encode_bytes_with_length(
    bytes: &[u8],
    enc: &mut impl strata_codec::Encoder,
) -> Result<(), CodecError> {
    let len = Varint::new(bytes.len() as u32)
        .ok_or(CodecError::MalformedField("length too large for varint"))?;
    len.encode(enc)?;
    enc.write_buf(bytes)?;
    Ok(())
}

/// Decodes raw bytes with a varint length prefix.
///
/// This reads a varint length prefix, then reads that many bytes and returns them as a Vec<u8>.
pub(crate) fn decode_bytes_with_length(dec: &mut impl strata_codec::Decoder) -> Result<Vec<u8>, CodecError> {
    let len_varint = Varint::decode(dec)?;
    let len = len_varint.inner() as usize;
    let mut bytes = vec![0u8; len];
    dec.read_buf(&mut bytes)?;
    Ok(bytes)
}
