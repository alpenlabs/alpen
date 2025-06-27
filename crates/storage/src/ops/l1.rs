//! L1 data operation interface.

use std::sync::Arc;

use strata_db::traits::*;
use strata_primitives::l1::{L1Block, L1BlockId, L1BlockManifest, L1Tx, L1TxRef};

use crate::exec::*;

inst_ops_simple! {
    (<D: L1Database> => L1DataOps) {
        put_block_data(mf: L1BlockManifest) => ();
        put_block_pending_validation(block: L1Block) => ();
        mark_block_valid(block_id: L1BlockId, height: u64, pow: [u8; 32]) => ();
        remove_invalid_block(block_id: L1BlockId) => ();
        // put_mmr_checkpoint(blockid: L1BlockId, mmr: CompactMmr) => ();
        set_canonical_chain_entry(height: u64, blockid: L1BlockId) => ();
        remove_canonical_chain_entries(start_height: u64, end_height: u64) => ();
        prune_to_height(height: u64) => ();
        get_canonical_chain_tip() => Option<(u64, L1BlockId)>;
        get_block_manifest(blockid: L1BlockId) => Option<L1BlockManifest>;
        get_block(blockid: L1BlockId) => Option<L1Block>;
        get_block_pow(blockid: L1BlockId) => Option<[u8; 32]>;
        get_best_valid_block_height() => Option<u64>;
        get_blocks_at_height_range(start: u64, end: u64) => Vec<L1BlockId>;
        get_canonical_blockid_at_height(height: u64) => Option<L1BlockId>;
        get_canonical_blockid_range(start_height: u64, end_height: u64) => Vec<L1BlockId>;
        get_block_txs(blockid: L1BlockId) => Option<Vec<L1TxRef>>;
        get_tx(tx_ref: L1TxRef) => Option<L1Tx>;
        // get_mmr(blockid: L1BlockId) => Option<CompactMmr>;
    }
}
