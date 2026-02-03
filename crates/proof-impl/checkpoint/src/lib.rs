//! This crate implements the final batch proof that aggregates both L1 Batch Proof and L2 Batch
//! Proof. It ensures that the previous batch proof was correctly settled on the L1
//! chain and that all L1-L2 transactions were processed.

use rkyv::rancor::Error as RkyvError;
use strata_checkpoint_types::{BatchTransition, ChainstateRootTransition};
use strata_proofimpl_cl_stf::program::ClStfOutput;
use zkaleido::ZkVmEnv;

pub mod program;

pub fn process_checkpoint_proof(zkvm: &impl ZkVmEnv, cl_stf_vk: &[u32; 8]) {
    let batches_count: usize = zkvm.read_serde();
    assert!(batches_count > 0);

    let first_output_buf = zkvm.read_verified_buf(cl_stf_vk);
    let ClStfOutput {
        epoch,
        initial_chainstate_root,
        mut final_chainstate_root,
    } = {
        // SAFETY: first_output_buf is read via `read_verified_buf` with the CL STF verifying key,
        // so it must be the exact rkyv output produced by the guest program.
        unsafe { rkyv::from_bytes_unchecked::<ClStfOutput, RkyvError>(&first_output_buf) }
    }
    .expect("rkyv deserialization failed");

    // Starting with 1 since we have already read the first CL STF output
    for _ in 1..batches_count {
        let cl_stf_output_buf = zkvm.read_verified_buf(cl_stf_vk);
        // SAFETY: cl_stf_output_buf is verified against the CL STF VK, so the bytes are trusted
        // rkyv output from the guest program.
        let cl_stf_output: ClStfOutput =
            unsafe { rkyv::from_bytes_unchecked::<ClStfOutput, RkyvError>(&cl_stf_output_buf) }
                .expect("rkyv deserialization failed");

        assert_eq!(
            cl_stf_output.initial_chainstate_root, final_chainstate_root,
            "continuity error"
        );

        assert_eq!(
            epoch, cl_stf_output.epoch,
            "transition must be within the same epoch"
        );

        final_chainstate_root = cl_stf_output.final_chainstate_root;
    }

    let chainstate_transition = ChainstateRootTransition {
        pre_state_root: initial_chainstate_root,
        post_state_root: final_chainstate_root,
    };

    let output = BatchTransition {
        epoch: epoch as u32,
        chainstate_transition,
    };

    let output_bytes = rkyv::to_bytes::<RkyvError>(&output).expect("rkyv serialization failed");
    zkvm.commit_buf(output_bytes.as_ref());
}
