//! L1 data operation interface.

use strata_asm_common::AsmManifest;
use strata_db_types::traits::*;
use strata_primitives::l1::L1BlockId;

use crate::{exec::*, instrumentation::components};

inst_ops_simple! {
    (<D: L1Database> => L1DataOps, component = components::STORAGE_L1) {
        put_block_data(manifest: AsmManifest) => ();
        // put_mmr_checkpoint(blockid: L1BlockId, mmr: CompactMmr) => ();
        set_canonical_chain_entry(height: u64, blockid: L1BlockId) => ();
        remove_canonical_chain_entries(start_height: u64, end_height: u64) => ();
        prune_to_height(height: u64) => ();
        get_canonical_chain_tip() => Option<(u64, L1BlockId)>;
        get_block_manifest(blockid: L1BlockId) => Option<AsmManifest>;
        get_canonical_blockid_at_height(height: u64) => Option<L1BlockId>;
        get_canonical_blockid_range(start_height: u64, end_height: u64) => Vec<L1BlockId>;
        // get_mmr(blockid: L1BlockId) => Option<CompactMmr>;
    }
}
