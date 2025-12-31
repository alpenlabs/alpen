//! Codec-compatible wrapper for `Vec<Vec<u8>>`
//!
//! TODO: Rethink this approach. Consider:
//! - Reusing existing RuntimeUpdateInput structure
//! - Using different serialization strategy
//! - Domain-specific types vs generic wrapper

use strata_codec::{Codec, CodecError, Decoder, Encoder};

/// Wrapper for Vec<Vec<u8>> with Codec implementation
#[derive(Debug, Clone)]
pub(crate) struct BytesList(pub Vec<Vec<u8>>);

impl Codec for BytesList {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        let outer_len = self.0.len() as u32;
        outer_len.encode(enc)?;
        for inner in &self.0 {
            let inner_len = inner.len() as u32;
            inner_len.encode(enc)?;
            enc.write_buf(inner)?;
        }
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let outer_len = u32::decode(dec)? as usize;
        let mut outer = Vec::with_capacity(outer_len);
        for _ in 0..outer_len {
            let inner_len = u32::decode(dec)? as usize;
            let mut inner = vec![0u8; inner_len];
            dec.read_buf(&mut inner)?;
            outer.push(inner);
        }
        Ok(BytesList(outer))
    }
}
