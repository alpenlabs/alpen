//! ASM data operation interface.

use strata_asm_common::AuxData;
use strata_db_types::traits::*;
use strata_primitives::l1::L1BlockCommitment;
use strata_state::asm_state::AsmState;

use crate::{exec::*, instrumentation::components};

inst_ops_simple! {
    (<D: AsmDatabase> => AsmDataOps, component = components::STORAGE_ASM) {
        put_asm_state(block: L1BlockCommitment, state: AsmState) => ();
        get_asm_state(block: L1BlockCommitment) => Option<AsmState>;
        get_latest_asm_state() => Option<(L1BlockCommitment, AsmState)>;
        get_asm_states_from(from_block: L1BlockCommitment, max_count: usize) => Vec<(L1BlockCommitment, AsmState)>;
        put_aux_data(block: L1BlockCommitment, data: AuxData) => ();
        get_aux_data(block: L1BlockCommitment) => Option<AuxData>;
    }
}
