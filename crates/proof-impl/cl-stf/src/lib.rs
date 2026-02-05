//! This crate implements the proof of the chain state transition function (STF) for L2 blocks,
//! verifying the correct state transitions as new L2 blocks are processed.

pub mod program;

use program::ClStfOutput;
use rkyv::rancor::Error as RkyvError;
use strata_chainexec::{ChainExecutor, MemExecContext};
use strata_chaintsn::context::L2HeaderAndParent;
use strata_codec_utils::decode_rkyv;
use strata_ol_chain_types::{
    check_block_credential, validate_block_structure, ExecSegment, L2Block, L2BlockHeader, L2Header,
};
use strata_ol_chainstate_types::Chainstate;
use strata_params::RollupParams;
use zkaleido::ZkVmEnv;

pub fn process_cl_stf(zkvm: &impl ZkVmEnv, el_vkey: &[u32; 8]) {
    // 1. Read the rollup params
    let rollup_params: RollupParams = zkvm.read_serde();

    // 2. Read the parent header which we consider valid and the initial chainstate from which we
    //    start the transition
    let parent_header_buf = zkvm.read_buf();
    let mut parent_header: L2BlockHeader =
        decode_rkyv(&parent_header_buf).expect("rkyv deserialization failed");
    let initial_chainstate_buf = zkvm.read_buf();
    let initial_chainstate: Chainstate =
        decode_rkyv(&initial_chainstate_buf).expect("rkyv deserialization failed");
    let mut ctx = MemExecContext::default();
    ctx.put_chainstate(parent_header.get_blockid(), initial_chainstate.clone());

    let initial_chainstate_root = initial_chainstate.compute_state_root();
    let mut final_chainstate_root = initial_chainstate_root;

    // 3. Read L2 blocks and parent header
    let l2_blocks_buf = zkvm.read_buf();
    let l2_blocks: Vec<L2Block> = decode_rkyv(&l2_blocks_buf).expect("rkyv deserialization failed");
    assert!(!l2_blocks.is_empty(), "At least one L2 block is required");

    // 4. Read the verified exec segments
    // This is the expected output of EVM EE STF Proof
    // Right now, each L2 block must contain exactly one ExecSegment, but this may change in the
    // future
    let exec_segments_buf = zkvm.read_verified_buf(el_vkey);
    let exec_segments: Vec<ExecSegment> =
        decode_rkyv(&exec_segments_buf).expect("rkyv deserialization failed");
    assert_eq!(
        l2_blocks.len(),
        exec_segments.len(),
        "mismatch len of l2 block and exec segments"
    );

    // NOTE: block range in cl-stf must not cross epoch boundaries
    let mut epoch = initial_chainstate.cur_epoch();

    for (l2_block, exec_segment) in l2_blocks.iter().zip(exec_segments) {
        // 6. Verify that the exec segment is the same that was proven
        assert_eq!(
            l2_block.exec_segment(),
            &exec_segment,
            "mismatch between exec segment at height {:?}",
            l2_block.header().slot()
        );

        // 8. Now that the L2 Block body is verified, check that the L2 Block header is consistent
        //    with the body
        assert!(
            validate_block_structure(l2_block).is_ok(),
            "block validation failed"
        );

        // 9. Verify that the block credential is valid
        assert!(
            check_block_credential(l2_block.header(), &rollup_params).is_ok(),
            "Block credential verification failed"
        );

        // 10. Apply the state transition
        let executor = ChainExecutor::new(rollup_params.clone());
        let header_and_parent = L2HeaderAndParent::new_simple(
            l2_block.header().header().clone(),
            parent_header.clone(),
        );
        let output = executor
            .execute_block(&header_and_parent, l2_block.body(), &ctx)
            .expect("failed to process L2 Block");
        parent_header = l2_block.header().header().clone();
        final_chainstate_root = *output.computed_state_root();

        ctx.put_chainstate(
            l2_block.header().get_blockid(),
            output.write_batch().new_toplevel_state().clone(),
        );

        epoch = output.write_batch().new_toplevel_state().cur_epoch();
    }

    let output = ClStfOutput {
        epoch: epoch as u64,
        initial_chainstate_root,
        final_chainstate_root,
    };

    let output_bytes = rkyv::to_bytes::<RkyvError>(&output).expect("rkyv serialization failed");
    zkvm.commit_buf(output_bytes.as_ref());
}
