//! Per-block accessed-state record + bytecode storage support.
//!
//! Produced by the `AccessedStateGenerator` exex (phase 2 of the EE prover
//! redesign) when reth commits a block: the exex re-executes the block
//! wrapped in a `CacheDBProvider` against the parent state, then writes
//! what the block *read* (accounts, slots, code hashes, ancestor heights
//! for BLOCKHASH) here, plus any newly-referenced bytecodes into the
//! sibling bytecode tree.
//!
//! Consumer: the acct-proof range-witness extractor. It unions the per-block
//! records of a batch's blocks into a single multiproof target set, then runs
//! the pre/post state multiproofs to assemble the batch witness. With this
//! cache in place, the extractor no longer has to re-execute blocks.

// TODO(trey): consider splitting AccessedAccount apart because it contains both key-like and
// value-like data

use serde::{Deserialize, Serialize};
use serde_bytes::ByteArray;

use super::address::{EvmAddress, EvmSlot};

/// Accessed-state captured during one block's execution.
///
/// Bytecodes are stored separately by code hash in the bytecode tree —
/// keep this record small; many chunks reference the same contracts.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AccessedStateRecord {
    /// Accounts the block read (and the storage slots, if any).
    accounts: Vec<AccessedAccount>,

    /// Code hashes referenced during execution. Resolve via
    /// [`crate::AccessedStateStore`] bytecode lookups (see
    /// `AccessedStateStore::get_bytecode`).
    bytecode_hashes: Vec<ByteArray<32>>,

    /// Ancestor block numbers queried via the EVM `BLOCKHASH` opcode.
    ancestor_block_numbers: Vec<u64>,
}

impl AccessedStateRecord {
    pub fn new(
        accounts: Vec<AccessedAccount>,
        bytecode_hashes: Vec<[u8; 32]>,
        ancestor_block_numbers: Vec<u64>,
    ) -> Self {
        Self {
            accounts,
            bytecode_hashes: bytecode_hashes.into_iter().map(ByteArray::new).collect(),
            ancestor_block_numbers,
        }
    }

    pub fn accounts(&self) -> &[AccessedAccount] {
        &self.accounts
    }

    /// Returns the referenced code hashes.
    ///
    /// The stored field wraps each hash so that serde encodes it as a byte string; callers
    /// should not have to know that.
    pub fn bytecode_hashes(&self) -> impl Iterator<Item = [u8; 32]> + '_ {
        self.bytecode_hashes.iter().map(|hash| hash.into_array())
    }

    pub fn ancestor_block_numbers(&self) -> &[u64] {
        &self.ancestor_block_numbers
    }
}

/// One account the block read, with the set of storage slots accessed.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AccessedAccount {
    /// 20-byte account address (alloy `Address` bytes).
    #[serde(with = "serde_bytes")]
    address: EvmAddress,

    /// 32-byte storage slot keys (alloy `B256` bytes).
    storage_slots: Vec<ByteArray<32>>,
}

impl AccessedAccount {
    pub fn new(address: EvmAddress, storage_slots: Vec<EvmSlot>) -> Self {
        Self {
            address,
            storage_slots: storage_slots.into_iter().map(ByteArray::new).collect(),
        }
    }

    pub fn address(&self) -> EvmAddress {
        self.address
    }

    /// Returns the accessed storage slot keys.
    ///
    /// The stored field wraps each key so that serde encodes it as a byte string; callers
    /// should not have to know that.
    pub fn storage_slots(&self) -> impl Iterator<Item = EvmSlot> + '_ {
        self.storage_slots.iter().map(|slot| slot.into_array())
    }
}
