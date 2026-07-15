//! Perf input for the alpen-acct SP1 guest.
//!
//! Drives the acct guest with a zero-chunk update. Non-empty chunk inputs now
//! require a real DA witness, so this benchmark intentionally measures the
//! minimal account-proof path rather than pretending an empty DA witness is
//! valid for an executed chunk.

use alpen_ee_da_types::DaWitness;
use rsp_primitives::genesis::Genesis;
use ssz::Encode;
use strata_bridge_params::BridgeParams;
use strata_codec::encode_to_vec;
use strata_ee_acct_runtime::EePrivateInput;
use strata_ee_acct_types::{EeAccountState, UpdateExtraData};
use strata_identifiers::Hash;
use strata_proofimpl_alpen_acct::{EeAcctProgram, EeAcctProofInput};
use strata_snark_acct_runtime::{IInnerState, PrivateInput as UpdatePrivateInput};
use strata_snark_acct_types::{LedgerRefs, ProofState, Seqno, UpdateOutputs, UpdateProofPubParams};
use tracing::info;
use zkaleido::{ExecutionSummary, ZkVmHost, ZkVmProgram};

fn prepare_input() -> EeAcctProofInput {
    info!("Preparing input for Alpen Acct (zero chunks)");

    let initial_blkid = Hash::zero();
    let initial_state = EeAccountState::new(initial_blkid, Hash::zero(), Vec::new(), Vec::new());
    let state_root = initial_state.compute_state_root();

    let extra_data =
        UpdateExtraData::new(initial_blkid, initial_state.last_exec_state_root(), 0, 0);
    let extra_data_bytes = encode_to_vec(&extra_data).expect("encode extra data");

    let pub_params = UpdateProofPubParams::new(
        Seqno::zero(),
        ProofState::new(state_root, 0),
        ProofState::new(state_root, 0),
        vec![],
        LedgerRefs::new_empty(),
        UpdateOutputs::new_empty(),
        extra_data_bytes,
    );

    let snark_acct_private_input =
        UpdatePrivateInput::new(pub_params, initial_state.as_ssz_bytes(), Vec::new());

    let ee_private_input = EePrivateInput::new(Vec::new(), Vec::new(), Vec::new());

    EeAcctProofInput {
        genesis: Genesis::Mainnet,
        ee_private_input,
        snark_acct_private_input,
        da_witness: DaWitness::empty(),
        bridge_params: BridgeParams::new(200_000_000, Some(1_000_000_000))
            .expect("valid bridge params"),
    }
}

pub(crate) fn gen_perf_report(host: &impl ZkVmHost) -> (String, ExecutionSummary) {
    info!("Generating execution summary for Alpen Acct");
    let input = prepare_input();
    let summary =
        <EeAcctProgram as ZkVmProgram>::execute(&input, host).expect("alpen-acct execution");
    (EeAcctProgram::name(), summary)
}

#[cfg(test)]
mod tests {
    use strata_predicate::PredicateKey;

    use super::*;

    #[test]
    fn test_alpen_acct_native_execution() {
        let input = prepare_input();
        // Native execution uses `always_accept` so empty proof bytes
        // on each `ChunkInput` pass the recursive verifier. SP1 mock
        // perf substitutes the real predicate at host-init time, so
        // the cycle count there reflects an honest verify.
        let program = EeAcctProgram::new(PredicateKey::always_accept());
        let result = program.execute(&input).expect("native execution");
        assert_eq!(
            result.cur_state().inner_state(),
            result.new_state().inner_state()
        );
    }
}
