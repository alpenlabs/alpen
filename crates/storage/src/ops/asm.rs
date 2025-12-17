//! ASM data operation interface.

use strata_db_types::traits::*;
use strata_primitives::{buf::Buf32, l1::L1BlockCommitment};
use strata_state::asm_state::AsmState;

use crate::exec::*;

inst_ops_simple! {
    (<D: AsmDatabase> => AsmDataOps) {
        put_asm_state(block: L1BlockCommitment, state: AsmState) => ();
        get_asm_state(block: L1BlockCommitment) => Option<AsmState>;
        get_latest_asm_state() => Option<(L1BlockCommitment, AsmState)>;
        get_asm_states_from(from_block: L1BlockCommitment, max_count: usize) => Vec<(L1BlockCommitment, AsmState)>;
        store_manifest_hash(index: u64, hash: Buf32) => ();
        get_manifest_hash(index: u64) => Option<Buf32>;
    }
}
