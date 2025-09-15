//! L1 data operation interface.

use std::sync::Arc;

use strata_asm_common::AnchorState;
use strata_asm_stf::AsmStfOutput;
use strata_db::traits::*;
use strata_primitives::l1::L1BlockCommitment;

use crate::exec::*;

inst_ops_simple! {
    (<D: AsmDatabase> => AsmDataOps) {
        put_asm_output(block: L1BlockCommitment, output: AsmStfOutput) => ();
        get_asm_output(block: L1BlockCommitment) => Option<AsmStfOutput>;
        get_latest_anchor_state() => Option<(L1BlockCommitment, AnchorState)>;
    }
}
