use strata_ee_acct_types::{
    EnvError, EnvResult, ExecBlock, ExecHeader, ExecPayload, ExecutionEnvironment,
};
use strata_ee_chain_types::{ChunkTransition, ExecInputs, ExecOutputs};

use crate::chunk::ChunkBlock;

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
    transition: &ChunkTransition,
) -> Result<()> {
    // TODO
    Ok(())
}
