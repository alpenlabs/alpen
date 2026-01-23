use strata_ee_acct_types::{
    EnvError, EnvResult, ExecBlock, ExecHeader, ExecPayload, ExecutionEnvironment,
};
use strata_ee_chain_types::{ChunkTransition, ExecInputs, ExecOutputs, SequenceTracker};

use crate::chunk::{Chunk, ChunkBlock};

/// Processes a block from a chunk with associated inputs, merging results into
/// the passed state.
pub fn process_block<E: ExecutionEnvironment>(
    ee: &E,
    state: &mut E::PartialState,
    block: &ChunkBlock<'_, E>,
) -> EnvResult<()> {
    // Repackage the block into the payload we can execute.
    let eb = block.exec_block();
    let header_intrinsics = eb.get_header().get_intrinsics();
    let epl = ExecPayload::new(&header_intrinsics, eb.get_body());

    // Execute the block, verify consistency.
    let exec_outp = ee.execute_block_body(state, &epl, block.inputs())?;
    ee.verify_outputs_against_header(eb.get_header(), &exec_outp)?;

    // Check that the outputs match the chunk block.
    if exec_outp.outputs() != block.outputs() {
        return Err(EnvError::InvalidBlock);
    }

    // Merge the changes and return the outputs.
    ee.merge_write_into_state(state, exec_outp.write_batch())?;

    Ok(())
}

/// Processes a chunk and updates the state, verifying the final state of the chunk and that it
/// matches the inputs/outputs described in the transition.
pub fn process_chunk_transition<E: ExecutionEnvironment>(
    ee: &E,
    state: &mut E::PartialState,
    chunk: &Chunk<'_, E>,
    tsn: &ChunkTransition,
) -> EnvResult<()> {
    // Set up the trackers.
    let mut deposits_tracker = SequenceTracker::new(tsn.inputs().subject_deposits());
    let mut out_msg_tracker = SequenceTracker::new(tsn.outputs().output_messages());
    let mut out_xfr_tracker = SequenceTracker::new(tsn.outputs().output_transfers());

    for cb in chunk.blocks() {
        // Verify the block itself.
        process_block(ee, state, cb)?;

        // Check the block's IO.
        deposits_tracker
            .consume_inputs(cb.inputs().subject_deposits())
            .map_err(|_| EnvError::InconsistentChunkIo)?;
        out_msg_tracker
            .consume_inputs(cb.outputs().output_messages())
            .map_err(|_| EnvError::InconsistentChunkIo)?;
        out_xfr_tracker
            .consume_inputs(cb.outputs().output_transfers())
            .map_err(|_| EnvError::InconsistentChunkIo)?;
    }

    // Make sure all the trackers are consumed.
    if !deposits_tracker.is_empty() || !out_msg_tracker.is_empty() || !out_xfr_tracker.is_empty() {
        return Err(EnvError::InconsistentChunkIo);
    }

    // TODO
    Ok(())
}
