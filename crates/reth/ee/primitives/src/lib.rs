//! Common primitive types for use in ee client

pub mod indexed_vec;

use alloy_primitives::{Address, FixedBytes, B256};
use bitcoin_bosd::Descriptor;

/// Represents an amount of native bitcoin token
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct BitcoinAmount {
    sats: u64,
}

impl BitcoinAmount {
    pub fn new_from_sats(sats: u64) -> Self {
        Self { sats }
    }

    pub fn sats(&self) -> u64 {
        self.sats
    }
}

/// Representation of a valid address in bitcoin
pub type BitcoinAddress = Descriptor;

/// Address of account in OL
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountAddress(FixedBytes<32>);

/// Address of an evm account in alpen ee
pub type EEAddress = Address;

/// Unique identifier of an L1 (Bitcoin) Block
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct L1BlockId(FixedBytes<32>);

impl From<FixedBytes<32>> for L1BlockId {
    fn from(value: FixedBytes<32>) -> Self {
        Self(value)
    }
}

/// Unique identifier of an OL Block
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OlBlockId(FixedBytes<32>);

impl From<FixedBytes<32>> for OlBlockId {
    fn from(value: FixedBytes<32>) -> Self {
        Self(value)
    }
}

/// Blockhash of an alpen ee block
pub type EEBlockHash = B256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct L1BlockCommitment {
    height: u64,
    blockid: L1BlockId,
}

impl L1BlockCommitment {
    pub fn new(height: u64, blockid: L1BlockId) -> Self {
        Self { height, blockid }
    }

    pub fn height(&self) -> u64 {
        self.height
    }

    pub fn blockid(&self) -> &L1BlockId {
        &self.blockid
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OlBlockCommitment {
    height: u64,
    blockid: OlBlockId,
}

impl OlBlockCommitment {
    pub fn new(height: u64, blockid: OlBlockId) -> Self {
        Self { height, blockid }
    }

    pub fn height(&self) -> u64 {
        self.height
    }

    pub fn blockid(&self) -> &OlBlockId {
        &self.blockid
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EEBlockCommitment {
    height: u64,
    blockhash: EEBlockHash,
}

impl EEBlockCommitment {
    pub fn new(height: u64, blockhash: EEBlockHash) -> Self {
        Self { height, blockhash }
    }

    pub fn height(&self) -> u64 {
        self.height
    }

    pub fn blockhash(&self) -> &EEBlockHash {
        &self.blockhash
    }
}
