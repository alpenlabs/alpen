//! Traits for the chain worker to interface with the underlying system.

use bitcoin::{Block, Network};
use strata_btc_types::BitcoinTxid;
use strata_primitives::prelude::*;
use strata_state::asm_state::AsmState;

use crate::WorkerResult;

/// Context trait for a worker to interact with the database and Bitcoin Client.
pub trait WorkerContext {
    /// Fetches a Bitcoin [`Block`] at a given height.
    fn get_l1_block(&self, blockid: &L1BlockId) -> WorkerResult<Block>;

    /// Fetches the [`AsmState`] given the block id.
    fn get_anchor_state(&self, blockid: &L1BlockCommitment) -> WorkerResult<AsmState>;

    /// Fetches the latest [`AsmState`] - the one that corresponds to the "highest" block.
    fn get_latest_asm_state(&self) -> WorkerResult<Option<(L1BlockCommitment, AsmState)>>;

    /// Puts the [`AsmState`] into DB.
    fn store_anchor_state(&self, blockid: &L1BlockCommitment, state: &AsmState)
    -> WorkerResult<()>;

    /// A Bitcoin network identifier.
    fn get_network(&self) -> WorkerResult<Network>;

    /// Fetches a raw Bitcoin transaction by txid.
    ///
    /// Returns the raw transaction bytes.
    fn get_bitcoin_tx(&self, txid: &BitcoinTxid) -> WorkerResult<RawBitcoinTx>;

    /// Appends a manifest hash to the MMR database and returns the leaf index.
    ///
    /// This should be called after each STF execution with the manifest hash.
    fn append_manifest_to_mmr(&self, manifest_hash: [u8; 32]) -> WorkerResult<u64>;

    /// Stores a manifest hash at the given MMR leaf index for fast lookup.
    fn store_manifest_hash(&self, index: u64, hash: [u8; 32]) -> WorkerResult<()>;

    /// Generates an MMR proof for the given leaf index.
    ///
    /// Returns a Merkle proof that can be used to verify the inclusion of the leaf.
    fn generate_mmr_proof(&self, index: u64) -> WorkerResult<strata_merkle::MerkleProofB32>;

    /// Retrieves a manifest hash by its MMR leaf index.
    ///
    /// Returns None if no manifest hash is stored at the given index.
    fn get_manifest_hash(&self, index: u64) -> WorkerResult<Option<[u8; 32]>>;
}
