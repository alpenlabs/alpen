//! ASM data operation interface.

use std::sync::Arc;

use strata_db::traits::*;
use strata_primitives::l1::L1BlockCommitment;
use strata_state::asm_state::AsmState;

use crate::exec::*;

inst_ops_simple! {
    (<D: AsmDatabase> => AsmDataOps) {
        put_asm_state(block: L1BlockCommitment, state: AsmState) => ();
        get_asm_state(block: L1BlockCommitment) => Option<AsmState>;
        get_latest_asm_state() => Option<(L1BlockCommitment, AsmState)>;
    }
}
