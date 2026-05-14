//! Per-block accessed-state record + bytecode storage support.
//!
//! Produced by the `AccessedStateGenerator` exex (phase 2 of the EE prover
//! redesign) when reth commits a block: the exex re-executes the block
//! wrapped in a `CacheDBProvider` against the parent state, then writes
//! what the block *read* (accounts, slots, code hashes, ancestor heights
//! for BLOCKHASH) here, plus any newly-referenced bytecodes into the
//! sibling bytecode tree.
//!
//! Consumer: the chunk-builder at chunk-seal time. It unions the per-block
//! records of the chunk's blocks into a single multiproof target set, then
//! runs the two pre/post state multiproofs and assembles the
//! `ChunkWitnessRecord`. With this cache in place, chunk-sealing no longer
//! has to re-execute blocks.

use borsh::{BorshDeserialize, BorshSerialize};

/// Accessed-state captured during one block's execution.
///
/// Bytecodes are stored separately by code hash in the bytecode tree —
/// keep this record small; many chunks reference the same contracts.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct AccessedStateRecord {
    /// Accounts the block read (and the storage slots, if any).
    pub accounts: Vec<AccessedAccount>,
    /// Code hashes referenced during execution. Resolve via
    /// [`crate::ChunkWitnessStore`]-adjacent bytecode lookups (see
    /// `AccessedStateStore::get_bytecode`).
    pub bytecode_hashes: Vec<[u8; 32]>,
    /// Ancestor block numbers queried via the EVM `BLOCKHASH` opcode.
    pub ancestor_block_numbers: Vec<u64>,
}

impl AccessedStateRecord {
    pub fn new(
        accounts: Vec<AccessedAccount>,
        bytecode_hashes: Vec<[u8; 32]>,
        ancestor_block_numbers: Vec<u64>,
    ) -> Self {
        Self {
            accounts,
            bytecode_hashes,
            ancestor_block_numbers,
        }
    }
}

/// One account the block read, with the set of storage slots accessed.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct AccessedAccount {
    /// 20-byte account address (alloy `Address` bytes).
    pub address: [u8; 20],
    /// 32-byte storage slot keys (alloy `B256` bytes).
    pub storage_slots: Vec<[u8; 32]>,
}

impl AccessedAccount {
    pub fn new(address: [u8; 20], storage_slots: Vec<[u8; 32]>) -> Self {
        Self {
            address,
            storage_slots,
        }
    }
}
