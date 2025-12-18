//! Serialized block package type for guest-side processing

use strata_codec::{Codec, CodecError, Decoder, Encoder};

/// Serialized block package containing execution metadata and raw block body.
///
/// This type represents a block in serialized form, combining two parts:
/// 1. **ExecBlockPackage** (SSZ-encoded): Contains block execution metadata including commitments
///    and hashes
/// 2. **Raw block body** (strata_codec-encoded): The actual block content/transactions
///
/// # Format
/// The internal bytes are laid out as:
/// ```text
/// [exec_block_package (SSZ)][raw_block_body (strata_codec)]
/// ```
///
/// # Usage
/// This type is used to pass serialized block data from the host to the guest zkVM.
/// The guest deserializes it into `CommitBlockData` which is then used to build
/// `CommitChainSegment` for state transition verification.
///
/// # Verification
/// When deserialized, the SHA256 hash of the raw block body is verified against
/// the commitment in the ExecBlockPackage to ensure data integrity.
#[derive(Debug, Clone)]
pub struct CommitBlockPackage(Vec<u8>);

impl CommitBlockPackage {
    /// Creates a new CommitBlockPackage from raw serialized bytes.
    ///
    /// # Parameters
    /// * `data` - Concatenated bytes of `[exec_block_package (SSZ)][raw_block_body (codec)]`
    pub fn new(data: Vec<u8>) -> Self {
        Self(data)
    }

    /// Gets a reference to the underlying serialized bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Consumes self and returns the underlying serialized bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }
}

impl Codec for CommitBlockPackage {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        // Encode length followed by data
        let len = self.0.len() as u32;
        len.encode(enc)?;
        enc.write_buf(&self.0)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        // Decode length then allocate and read data
        let len = u32::decode(dec)? as usize;
        let mut data = vec![0u8; len];
        dec.read_buf(&mut data)?;
        Ok(Self(data))
    }
}
