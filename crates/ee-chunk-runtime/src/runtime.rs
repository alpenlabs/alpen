//! Toplevel proof logic using no-copy input types..

use strata_ee_acct_types::{EnvError, EnvResult, ExecutionEnvironment};

use crate::{ArchivedPrivateInput, Chunk, ChunkBlock, verify_chunk_transition};

/// Verifies the private input's consistency using the provided execution
/// environment.
pub fn verify_input<E: ExecutionEnvironment>(
    ee: &E,
    input: &ArchivedPrivateInput,
) -> EnvResult<()> {
    // 1. Parse the various basic inputs.
    // FIXME better error variants here
    let tsn = input
        .try_decode_chunk_transition()
        .map_err(|_| EnvError::MalformedChainSegment)?;

    let prev_header = input
        .try_decode_prev_header::<E>()
        .map_err(|_| EnvError::MalformedCoinput)?;

    let mut pre_state = input
        .try_decode_pre_state::<E>()
        .map_err(|_| EnvError::MalformedCoinput)?;

    // 2. Parse the blocks into a chunk we can execute.
    // TODO rework borrowings here because this is really ugly
    let mut block_inputs = Vec::new();
    let mut block_outputs = Vec::new();
    for b in input.raw_chunk().blocks() {
        block_inputs.push(
            b.try_decode_exec_inputs()
                .map_err(|_| EnvError::MalformedChainSegment)?,
        );
        block_outputs.push(
            b.try_decode_exec_outputs()
                .map_err(|_| EnvError::MalformedChainSegment)?,
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

    // 3. Verify the chunk against the pre state.
    verify_chunk_transition(&tsn, ee, &prev_header, &mut pre_state, &chunk)?;

    Ok(())
}
