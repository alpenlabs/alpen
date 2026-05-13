//! Perf input for the alpen-acct SP1 guest.
//!
//! Drives the acct guest with a real chunk built from the canonical EVM
//! witness fixture (the same one `alpen_chunk` uses). The chunk runs
//! natively here — its public values feed into the acct's
//! `EePrivateInput.chunks`, and the acct verifies one chunk transition,
//! advances the EE tip blkid, and commits the resulting
//! `UpdateProofPubParams`.
//!
//! Cycle counts thus include: (a) recursive verification of the chunk
//! pubvals, (b) chunk continuity checks, (c) state-root recomputation,
//! (d) pubvals SSZ commit.

use ssz::Encode;
use strata_acct_types::BitcoinAmount;
use strata_codec::encode_to_vec;
use strata_ee_acct_runtime::{ChunkInput, EePrivateInput};
use strata_ee_acct_types::{EeAccountState, ExecBlock, ExecHeader, UpdateExtraData};
use strata_evm_ee::EvmExecutionEnvironment;
use strata_proofimpl_alpen_acct::{EeAcctProgram, EeAcctProofInput};
use strata_proofimpl_alpen_chunk::EeChunkProgram;
use strata_snark_acct_runtime::{IInnerState, PrivateInput as UpdatePrivateInput};
use strata_snark_acct_types::{LedgerRefs, ProofState, UpdateOutputs, UpdateProofPubParams};
use tracing::info;
use zkaleido::{ExecutionSummary, ZkVmHost, ZkVmProgram};

use super::alpen_chunk;

fn prepare_input() -> EeAcctProofInput {
    info!("Preparing input for Alpen Acct (one EVM chunk)");

    // 1. Run the chunk natively to get a concrete ChunkTransition.
    let chunk_input = alpen_chunk::prepare_input();
    let genesis = chunk_input.genesis.clone();
    let chunk_transition = EeChunkProgram::execute(&chunk_input).expect("chunk native execution");

    // 2. Pre-state: an empty EE account whose `last_exec_blkid` points to the chunk's parent block,
    //    so the chunk's parent → tip transition is valid against it.
    let parent_blkid = chunk_transition.parent_exec_blkid();

    let parent_state_root = chunk_input
        .private_input
        .try_decode_prev_header::<EvmExecutionEnvironment>()
        .expect("decode parent header")
        .get_state_root();

    let tip_state_root = chunk_input
        .private_input
        .raw_chunk()
        .blocks()
        .last()
        .expect("chunk should contain at least one block")
        .try_decode_block::<EvmExecutionEnvironment>()
        .expect("decode tip block")
        .get_header()
        .get_state_root();
    let initial_state = EeAccountState::new(
        parent_blkid,
        parent_state_root,
        BitcoinAmount::from_sat(0),
        Vec::new(),
        Vec::new(),
    );
    let pre_root = initial_state.compute_state_root();

    // 3. Post-state: same EE account but tip advanced to the chunk's output. This is what the acct
    //    guest's `pre_finalize_state` arrives at when there are no messages.
    let mut post_state = initial_state.clone();
    post_state.set_last_exec_blkid(chunk_transition.tip_exec_blkid());
    post_state.set_last_exec_state_root(tip_state_root);
    let post_root = post_state.compute_state_root();

    // 4. Build pubvals matching the post-state. The witness fixture is a vanilla Ethereum block —
    //    it doesn't touch the EE precompiles that emit subject deposits or output messages, so all
    //    the aggregate fields are empty.
    let extra_data = UpdateExtraData::new(chunk_transition.tip_exec_blkid(), tip_state_root, 0, 0);
    let extra_data_bytes = encode_to_vec(&extra_data).expect("encode extra data");

    let pub_params = UpdateProofPubParams::new(
        ProofState::new(pre_root, 0),
        ProofState::new(post_root, 0),
        vec![],
        LedgerRefs::new_empty(),
        UpdateOutputs::new_empty(),
        extra_data_bytes,
    );

    let update_private_input =
        UpdatePrivateInput::new(pub_params, initial_state.as_ssz_bytes(), Vec::new());

    // 5. EE private input: a single chunk with empty proof bytes. `always_accept` predicate doesn't
    //    verify the bytes; SP1 mock perf substitutes the real predicate at host-init time.
    let chunk_inputs = vec![ChunkInput::new(chunk_transition, Vec::new())];
    let ee_private_input = EePrivateInput::new(Vec::new(), Vec::new(), chunk_inputs);

    EeAcctProofInput {
        genesis,
        ee_private_input,
        update_private_input,
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
        // Pre and post state roots differ because the chunk advances
        // the EE tip blkid (a member of `EeAccountState`), which in
        // turn perturbs the SSZ-tree-hashed state root.
        assert_ne!(
            result.cur_state().inner_state(),
            result.new_state().inner_state()
        );
    }
}
