//! Builder for raw OP_RETURN scripts (non-SPS-50 format).

use bitcoin::{
    opcodes::all::OP_RETURN,
    script::{Builder, PushBytesBuf},
    ScriptBuf,
};

/// Builder for raw OP_RETURN scripts.
///
/// Use this when the data format is not SPS-50 (subprotocol/tx_type).
/// Provides a fluent API for constructing OP_RETURN scripts with arbitrary data.
///
/// # Example
///
/// ```ignore
/// let op_return = RawOpReturnBuilder::with_tag(*b"EEDA")
///     .push_byte(0x01)        // type = DA
///     .push_bytes(&prev_wtxid)
///     .build()
///     .expect("37 bytes fits in 80");
/// ```
#[derive(Clone, Debug)]
pub struct RawOpReturnBuilder {
    data: Vec<u8>,
}

impl RawOpReturnBuilder {
    /// Maximum OP_RETURN data size (standardness rule).
    pub const MAX_DATA_SIZE: usize = 80;

    /// Creates a new builder with the given tag bytes.
    pub fn with_tag(tag: [u8; 4]) -> Self {
        let mut data = Vec::with_capacity(Self::MAX_DATA_SIZE);
        data.extend_from_slice(&tag);
        Self { data }
    }

    /// Creates a new empty builder.
    pub fn new() -> Self {
        Self {
            data: Vec::with_capacity(Self::MAX_DATA_SIZE),
        }
    }

    /// Appends a single byte.
    pub fn push_byte(mut self, byte: u8) -> Self {
        self.data.push(byte);
        self
    }

    /// Appends raw bytes.
    pub fn push_bytes(mut self, bytes: &[u8]) -> Self {
        self.data.extend_from_slice(bytes);
        self
    }

    /// Returns the current data length.
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Returns true if the builder contains no data.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Builds the OP_RETURN script.
    ///
    /// Returns `Err` if data exceeds [`Self::MAX_DATA_SIZE`] (80 bytes).
    pub fn build(self) -> Result<ScriptBuf, &'static str> {
        if self.data.len() > Self::MAX_DATA_SIZE {
            return Err("OP_RETURN data exceeds 80 bytes");
        }
        // Convert to PushBytesBuf - safe because we already validated size <= 80 bytes
        // (well under the 520 byte PushBytes limit)
        let push_bytes = PushBytesBuf::try_from(self.data)
            .expect("80 bytes is always valid for PushBytesBuf");
        Ok(Builder::new()
            .push_opcode(OP_RETURN)
            .push_slice(push_bytes)
            .into_script())
    }
}

impl Default for RawOpReturnBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_da_op_return() {
        let prev_wtxid = [0x42u8; 32];
        let op_return = RawOpReturnBuilder::with_tag(*b"EEDA")
            .push_byte(0x01) // type = DA
            .push_bytes(&prev_wtxid)
            .build()
            .expect("should build");

        // Expected: OP_RETURN PUSH(37 bytes: tag(4) + type(1) + prev_wtxid(32))
        assert_eq!(op_return.len(), 1 + 1 + 37); // OP_RETURN + PUSH opcode + 37 bytes
    }

    #[test]
    fn test_max_size() {
        let mut builder = RawOpReturnBuilder::new();
        for _ in 0..80 {
            builder = builder.push_byte(0x00);
        }
        assert!(builder.build().is_ok());

        let mut builder = RawOpReturnBuilder::new();
        for _ in 0..81 {
            builder = builder.push_byte(0x00);
        }
        assert!(builder.build().is_err());
    }

    #[test]
    fn test_empty_builder() {
        let builder = RawOpReturnBuilder::new();
        assert!(builder.is_empty());
        assert_eq!(builder.len(), 0);
    }
}
