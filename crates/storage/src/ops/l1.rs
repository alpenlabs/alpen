//! L1 data operation interface.

use strata_asm_common::AsmManifest;
use strata_db_types::traits::*;
use strata_primitives::{l1::L1BlockId, L1Height};

use crate::{exec::*, instrumentation::components};

inst_ops_simple! {
    (<D: L1Database> => L1DataOps, component = components::STORAGE_L1) {
        put_block_data(manifest: AsmManifest) => ();
        // put_mmr_checkpoint(blockid: L1BlockId, mmr: CompactMmr) => ();
        set_canonical_chain_entry(height: L1Height, blockid: L1BlockId) => ();
        remove_canonical_chain_entries(start_height: L1Height, end_height: L1Height) => ();
        prune_to_height(height: L1Height) => ();
        get_canonical_chain_tip() => Option<(L1Height, L1BlockId)>;
        get_block_manifest(blockid: L1BlockId) => Option<AsmManifest>;
        get_canonical_blockid_at_height(height: L1Height) => Option<L1BlockId>;
        get_canonical_blockid_range(start_height: L1Height, end_height: L1Height) => Vec<L1BlockId>;
        // get_mmr(blockid: L1BlockId) => Option<CompactMmr>;
    }
}
