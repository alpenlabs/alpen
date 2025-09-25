use std::fmt;

use arbitrary::Arbitrary;
use bitcoin::{hashes::Hash, BlockHash};
use borsh::{BorshDeserialize, BorshSerialize};
use const_hex as hex;
use serde::{Deserialize, Serialize};

use crate::{buf::Buf32, hash::sha256d};

/// ID of an L1 block, usually the hash of its header.
#[derive(
    Copy,
    Clone,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    Default,
    Arbitrary,
    BorshSerialize,
    BorshDeserialize,
    Deserialize,
    Serialize,
)]
pub struct L1BlockId(Buf32);

impl L1BlockId {
    /// Computes the [`L1BlockId`] from the header buf. This is expensive in proofs and
    /// should only be done when necessary.
    pub fn compute_from_header_buf(buf: &[u8]) -> L1BlockId {
        Self::from(sha256d(buf))
    }
}

// Custom implementation without Debug/Display to avoid conflicts
impl From<Buf32> for L1BlockId {
    fn from(value: Buf32) -> Self {
        Self(value)
    }
}

impl From<L1BlockId> for Buf32 {
    fn from(value: L1BlockId) -> Self {
        value.0
    }
}

impl AsRef<[u8; 32]> for L1BlockId {
    fn as_ref(&self) -> &[u8; 32] {
        self.0.as_ref()
    }
}

impl From<BlockHash> for L1BlockId {
    fn from(value: BlockHash) -> Self {
        L1BlockId(value.into())
    }
}

impl From<L1BlockId> for BlockHash {
    fn from(value: L1BlockId) -> Self {
        BlockHash::from_byte_array(value.0.into())
    }
}

#[derive(
    Copy,
    Clone,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    Default,
    Arbitrary,
    BorshDeserialize,
    BorshSerialize,
    Deserialize,
    Serialize,
)]
pub struct L1BlockCommitment {
    height: u64,
    blkid: L1BlockId,
}

impl fmt::Display for L1BlockCommitment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Show first 2 and last 2 bytes of block ID (4 hex chars each)
        let blkid_bytes = self.blkid.as_ref();
        let first_2 = &blkid_bytes[..2];
        let last_2 = &blkid_bytes[30..];

        let mut first_hex = [0u8; 4];
        let mut last_hex = [0u8; 4];
        hex::encode_to_slice(first_2, &mut first_hex)
            .expect("Failed to encode first 2 bytes to hex");
        hex::encode_to_slice(last_2, &mut last_hex).expect("Failed to encode last 2 bytes to hex");

        write!(
            f,
            "{}@{}..{}",
            self.height,
            std::str::from_utf8(&first_hex)
                .expect("Failed to convert first 2 hex bytes to UTF-8 string"),
            std::str::from_utf8(&last_hex)
                .expect("Failed to convert last 2 hex bytes to UTF-8 string")
        )
    }
}

impl fmt::Debug for L1BlockCommitment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "L1BlockCommitment(height={}, blkid={:?})",
            self.height, self.blkid
        )
    }
}

impl L1BlockCommitment {
    pub fn new(height: u64, blkid: L1BlockId) -> Self {
        Self { height, blkid }
    }

    pub fn height(&self) -> u64 {
        self.height
    }

    pub fn blkid(&self) -> &L1BlockId {
        &self.blkid
    }
}

// Custom debug implementation to print the block hash in little endian
impl fmt::Debug for L1BlockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut bytes = self.0 .0;
        bytes.reverse();
        let hex_str = hex::encode(bytes);
        f.write_str(&hex_str)
    }
}

// Custom display implementation to print the block hash in little endian
impl fmt::Display for L1BlockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut bytes = self.0 .0;
        bytes.reverse();
        let hex_str = hex::encode(bytes);
        f.write_str(&hex_str)
    }
}
