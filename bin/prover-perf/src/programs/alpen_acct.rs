//! Perf input for the alpen-acct SP1 guest.
//!
//! Mirrors the zero-chunks setup in `alpen-acct`'s
//! `test_native_acct_execution_zero_chunks`: minimal account state with
//! no chunks, no DA, no messages. Exercises the guest's input
//! deserialisation + verify-and-process path without paying the cycle
//! cost of recursive chunk-proof verification, so the cycle count
//! tracks the floor of the acct guest's overhead.
//!
//! Once Alpen has a stable rich e2e fixture (real chunks, real DA),
//! this should be replaced with that input shape so the perf number
//! reflects realistic batches.

use rsp_primitives::genesis::Genesis;
use ssz::Encode;
use strata_acct_types::{BitcoinAmount, Hash};
use strata_codec::encode_to_vec;
use strata_ee_acct_runtime::EePrivateInput;
use strata_ee_acct_types::{EeAccountState, UpdateExtraData};
use strata_proofimpl_alpen_acct::{EeAcctProgram, EeAcctProofInput};
use strata_snark_acct_runtime::{Coinput, IInnerState, PrivateInput as UpdatePrivateInput};
use strata_snark_acct_types::{LedgerRefs, ProofState, UpdateOutputs, UpdateProofPubParams};
use tracing::info;
use zkaleido::{PerformanceReport, ZkVmHostPerf, ZkVmProgramPerf};

fn prepare_input() -> EeAcctProofInput {
    info!("Preparing input for Alpen Acct (zero chunks)");

    let initial_blkid = Hash::zero();
    let initial_state = EeAccountState::new(
        initial_blkid,
        BitcoinAmount::from_sat(0),
        Vec::new(),
        Vec::new(),
    );
    let state_root = initial_state.compute_state_root();

    let extra_data = UpdateExtraData::new(initial_blkid, 0, 0);
    let extra_data_bytes = encode_to_vec(&extra_data).expect("encode extra data");

    let pub_params = UpdateProofPubParams::new(
        ProofState::new(state_root, 0),
        ProofState::new(state_root, 0),
        vec![],
        LedgerRefs::new_empty(),
        UpdateOutputs::new_empty(),
        extra_data_bytes,
    );

    let coinputs: Vec<Coinput> = pub_params
        .message_inputs()
        .iter()
        .map(|_| Coinput::new(Vec::new()))
        .collect();
    let update_private_input =
        UpdatePrivateInput::new(pub_params, initial_state.as_ssz_bytes(), coinputs);
    let ee_private_input = EePrivateInput::new(Vec::new(), Vec::new(), Vec::new());

    EeAcctProofInput {
        genesis: Genesis::Mainnet,
        ee_private_input,
        update_private_input,
    }
}

pub(crate) fn gen_perf_report(host: &impl ZkVmHostPerf) -> PerformanceReport {
    info!("Generating performance report for Alpen Acct");
    let input = prepare_input();
    EeAcctProgram::perf_report(&input, host).unwrap()
}

#[cfg(test)]
mod tests {
    use strata_predicate::{PredicateKey, PredicateTypeId};

    use super::*;

    #[test]
    fn test_alpen_acct_native_execution() {
        let input = prepare_input();
        // Predicate key isn't evaluated under the zero-chunks input but
        // the program still requires one; use a real (Schnorr) shape so
        // we don't accidentally short-circuit through `always_accept`.
        let program = EeAcctProgram::new(PredicateKey::new(
            PredicateTypeId::Bip340Schnorr,
            vec![0u8; 32],
        ));
        let result = program.execute(&input).expect("native execution");
        assert_eq!(
            result.cur_state().inner_state(),
            result.new_state().inner_state()
        );
    }
}
