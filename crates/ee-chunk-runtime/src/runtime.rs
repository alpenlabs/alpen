//! Toplevel proof logic using no-copy input types.

use strata_ee_acct_types::{EnvError, EnvResult, ExecutionEnvironment};

use crate::{ArchivedPrivateInput, Chunk, ChunkBlock, verify_chunk_transition};

/// Verifies the private input's consistency using the provided execution
/// environment.
pub fn verify_input<E: ExecutionEnvironment>(
    ee: &E,
    input: &ArchivedPrivateInput,
) -> EnvResult<()> {
    // 1. Parse the various basic inputs.
    let tsn = input
        .try_decode_chunk_transition()
        .map_err(|_| EnvError::MalformedChainSegment)?;

    // FIXME(STR-3685): do we actually need the header or just the blkid+state?
    let prev_header = input
        .try_decode_prev_header::<E>()
        .map_err(|_| EnvError::MalformedChainSegment)?;

    // 2. Parse the blocks (with their per-block witnesses) into a chunk we can execute. Each block
    //    carries its own depth-0 partial-state witness, decoded here into a parallel `block_states`
    //    list.
    // TODO(STR-3685): rework borrowings here because this is really ugly
    let mut block_inputs = Vec::new();
    let mut block_outputs = Vec::new();
    let mut block_states = Vec::new();
    for b in input.raw_chunk().blocks() {
        block_inputs.push(
            b.try_decode_exec_inputs()
                .map_err(|_| EnvError::MalformedChainSegment)?,
        );
        block_outputs.push(
            b.try_decode_exec_outputs()
                .map_err(|_| EnvError::MalformedChainSegment)?,
        );
        block_states.push(
            b.try_decode_partial_state::<E>()
                .map_err(|_| EnvError::MalformedChainState)?,
        );
    }

    let mut blocks = Vec::new();
    for (i, b) in input.raw_chunk().blocks().iter().enumerate() {
        let block = b
            .try_decode_block::<E>()
            .map_err(|_| EnvError::MalformedChainSegment)?;
        blocks.push(ChunkBlock::new(&block_inputs[i], &block_outputs[i], block));
    }

    let chunk = Chunk::<'_, E>::new(blocks);

    // 3. Verify the chunk as a verified chain of per-block transitions.
    verify_chunk_transition(&tsn, ee, &prev_header, &mut block_states, &chunk)?;

    Ok(())
}
