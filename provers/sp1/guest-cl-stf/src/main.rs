use sha2::{Digest, Sha256};
use strata_primitives::params::RollupParams;
use strata_proofimpl_checkpoint::{ChainstateSnapshot, L2BatchProofOutput};
use strata_proofimpl_cl_stf::{verify_and_transition, Chainstate, L2Block};
use strata_proofimpl_evm_ee_stf::ELProofPublicParams;

mod vks;

fn main() {
    let rollup_params: RollupParams = sp1_zkvm::io::read();

    let input_raw = sp1_zkvm::io::read_vec();
    let (prev_state, block): (Chainstate, L2Block) = borsh::from_slice(&input_raw).unwrap();

    // Verify the EL proof
    let el_vkey = vks::GUEST_EVM_EE_STF_ELF_ID;
    let el_pp_raw = sp1_zkvm::io::read_vec();
    let el_pp_raw_digest = Sha256::digest(&el_pp_raw);
    sp1_zkvm::lib::verify::verify_sp1_proof(el_vkey, &el_pp_raw_digest.into());
    let el_pp_deserialized: ELProofPublicParams = bincode::deserialize(&el_pp_raw).unwrap();

    let new_state = verify_and_transition(
        prev_state.clone(),
        block,
        el_pp_deserialized,
        &rollup_params,
    );

    let initial_snapshot = ChainstateSnapshot {
        hash: prev_state.compute_state_root(),
        slot: prev_state.chain_tip_slot(),
        l2_blockid: prev_state.chain_tip_blockid(),
    };

    let final_snapshot = ChainstateSnapshot {
        hash: new_state.compute_state_root(),
        slot: new_state.chain_tip_slot(),
        l2_blockid: new_state.chain_tip_blockid(),
    };

    let cl_stf_public_params = L2BatchProofOutput {
        // TODO: Accumulate the deposits
        deposits: Vec::new(),
        final_snapshot,
        initial_snapshot,
        rollup_params_commitment: rollup_params.compute_hash(),
    };

    sp1_zkvm::io::commit_slice(&borsh::to_vec(&cl_stf_public_params).unwrap());
}
