use strata_ee_acct_types::{
    EnvError, EnvResult, ExecBlock, ExecHeader, ExecPartialState, ExecPayload,
    ExecutionEnvironment, Hash,
};
use strata_ee_chain_types::{
    ChunkTransition, ExecInputs, ExecOutputs, OutputMessage, OutputTransfer, SequenceTracker,
    SubjectDepositData,
};

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

struct IoTracker<'c> {
    deposits_tracker: SequenceTracker<'c, SubjectDepositData>,
    out_msg_tracker: SequenceTracker<'c, OutputMessage>,
    out_xfr_tracker: SequenceTracker<'c, OutputTransfer>,
}

impl<'c> IoTracker<'c> {
    fn from_io(expected_inputs: &'c ExecInputs, expected_outputs: &'c ExecOutputs) -> Self {
        Self {
            deposits_tracker: SequenceTracker::new(expected_inputs.subject_deposits()),
            out_msg_tracker: SequenceTracker::new(expected_outputs.output_messages()),
            out_xfr_tracker: SequenceTracker::new(expected_outputs.output_transfers()),
        }
    }

    /// Processes a pair of inputs and outputs, verifying they're all correct.
    fn check_update(&mut self, inps: &ExecInputs, outps: &ExecOutputs) -> EnvResult<()> {
        // Check them first.
        self.deposits_tracker
            .check_inputs(inps.subject_deposits())
            .map_err(|_| EnvError::InconsistentChunkIo)?;
        self.out_msg_tracker
            .check_inputs(outps.output_messages())
            .map_err(|_| EnvError::InconsistentChunkIo)?;
        self.out_xfr_tracker
            .check_inputs(outps.output_transfers())
            .map_err(|_| EnvError::InconsistentChunkIo)?;

        // And then advance them after they've all been checked.
        self.deposits_tracker
            .advance_unchecked(inps.subject_deposits().len());
        self.out_msg_tracker
            .advance_unchecked(outps.output_messages().len());
        self.out_xfr_tracker
            .advance_unchecked(outps.output_transfers().len());

        Ok(())
    }

    fn is_all_consumed(&self) -> bool {
        self.deposits_tracker.is_fully_consumed()
            && self.out_msg_tracker.is_fully_consumed()
            && self.out_xfr_tracker.is_fully_consumed()
    }

    fn verify_all_consumed(&self) -> EnvResult<()> {
        if self.is_all_consumed() {
            Ok(())
        } else {
            Err(EnvError::InconsistentChunkIo)
        }
    }
}

/// Processes a chunk's blocks and updates the state, checking the IO against an
/// expected IO trace.
fn process_chunk_blocks<E: ExecutionEnvironment>(
    ee: &E,
    state: &mut E::PartialState,
    chunk: &Chunk<'_, E>,
    verified_tip: Hash,
    expected_inputs: &ExecInputs,
    expected_outputs: &ExecOutputs,
) -> EnvResult<()> {
    // 1. Check that the chunk is nonempty.
    if chunk.blocks().is_empty() {
        return Err(EnvError::MalformedChainSegment);
    }

    // 2. Process each block, tracking the IO traces and chain continuity.
    let mut io_tracker = IoTracker::from_io(expected_inputs, expected_outputs);
    let mut cur_verified_tip_blkid = verified_tip;
    for cb in chunk.blocks() {
        let header = cb.exec_block().get_header();

        // Verify it builds on the previous block.
        if header.get_parent_id() != cur_verified_tip_blkid {
            return Err(EnvError::MismatchedChainSegment);
        }

        // Verify the block itself.
        process_block(ee, state, cb)?;

        // Check the block's IO.
        io_tracker.check_update(cb.inputs(), cb.outputs())?;

        cur_verified_tip_blkid = header.compute_block_id();
    }

    // 3. Make sure all the trackers are consumed.
    io_tracker.verify_all_consumed()?;

    Ok(())
}

/// Verifies a chunk transition using the pre state, parent header, etc.
pub fn verify_chunk_transition<E: ExecutionEnvironment>(
    tsn: &ChunkTransition,
    ee: &E,
    prev_header: &<E::Block as ExecBlock>::Header,
    state: &mut E::PartialState,
    chunk: &Chunk<'_, E>,
) -> EnvResult<()> {
    // 1. Make sure the parent block ID we have that we're extending from
    // matches the chunk transition.
    let computed_prev_blkid = prev_header.compute_block_id();
    if computed_prev_blkid != tsn.parent_exec_blkid() {
        // TODO better error type?
        return Err(EnvError::MismatchedChainSegment);
    }

    // 2. Make sure the chunk is nonempty and check that the last block matches
    // the chunk transition.
    let Some(new_tip_header) = chunk.blocks().last().map(|b| b.exec_block().get_header()) else {
        return Err(EnvError::MalformedChainSegment);
    };

    let computed_new_tip_blkid = new_tip_header.compute_block_id();
    if computed_new_tip_blkid != tsn.tip_exec_blkid() {
        return Err(EnvError::MismatchedChainSegment);
    }

    // 2. Make sure the state matches the parent block's state root.
    let computed_pre_sr = state.compute_state_root()?;
    if computed_pre_sr != prev_header.get_state_root() {
        return Err(EnvError::MismatchedCurStateData);
    }

    // 3. Execute the blocks in the chunk.  This dooesn't verify the
    // intermediate state roots because that's expensive and we only really care
    // about the final state.
    process_chunk_blocks(
        ee,
        state,
        chunk,
        tsn.parent_exec_blkid(),
        tsn.inputs(),
        tsn.outputs(),
    )?;

    // 4. Compute the final state root and make sure it matches.
    let computed_post_sr = state.compute_state_root()?;
    if computed_post_sr != new_tip_header.get_state_root() {
        return Err(EnvError::MismatchedChainSegment);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use strata_ee_acct_types::{
        BlockAssembler, ExecBlock, ExecBlockOutput, ExecHeader, ExecPayload,
    };
    use strata_ee_chain_types::{ExecInputs, ExecOutputs};
    use strata_simple_ee::{
        SimpleBlock, SimpleBlockBody, SimpleExecutionEnvironment, SimpleHeader,
        SimpleHeaderIntrinsics, SimplePartialState, SimpleTransaction,
    };

    use super::*;
    use crate::chunk::{Chunk, ChunkBlock};

    fn alice() -> strata_acct_types::SubjectId {
        strata_acct_types::SubjectId::from([1u8; 32])
    }

    fn bob() -> strata_acct_types::SubjectId {
        strata_acct_types::SubjectId::from([2u8; 32])
    }

    /// Builds a valid SimpleBlock by executing the body against the given state,
    /// returning the block, its inputs, outputs, and the post-state.
    fn build_block(
        ee: &SimpleExecutionEnvironment,
        state: &SimplePartialState,
        parent_blkid: Hash,
        index: u64,
        body: SimpleBlockBody,
        inputs: ExecInputs,
    ) -> (SimpleBlock, ExecInputs, ExecOutputs, SimplePartialState) {
        let intrinsics = SimpleHeaderIntrinsics {
            parent_blkid,
            index,
        };
        let payload = ExecPayload::new(&intrinsics, &body);
        let output: ExecBlockOutput<SimpleExecutionEnvironment> =
            ee.execute_block_body(state, &payload, &inputs).unwrap();

        let header = ee.complete_header(&payload, &output).unwrap();
        let block = SimpleBlock::new(header, body);

        let mut post_state = state.clone();
        ee.merge_write_into_state(&mut post_state, output.write_batch())
            .unwrap();

        let outputs = output.outputs().clone();
        (block, inputs, outputs, post_state)
    }

    #[test]
    fn test_process_chunk_blocks_multi_block() {
        let ee = SimpleExecutionEnvironment;

        // Initial state: alice has 1000.
        let mut accounts = BTreeMap::new();
        accounts.insert(alice(), 1000);
        let initial_state = SimplePartialState::new(accounts);

        let genesis_header = SimpleHeader::genesis();
        let genesis_blkid = genesis_header.compute_block_id();

        // Block 1: alice -> bob 200
        let (block1, inp1, out1, state1) = build_block(
            &ee,
            &initial_state,
            genesis_blkid,
            1,
            SimpleBlockBody::new(vec![SimpleTransaction::Transfer {
                from: alice(),
                to: bob(),
                value: 200,
            }]),
            ExecInputs::new_empty(),
        );
        let blkid1 = block1.get_header().compute_block_id();

        // Block 2: alice -> bob 300
        let (block2, inp2, out2, state2) = build_block(
            &ee,
            &state1,
            blkid1,
            2,
            SimpleBlockBody::new(vec![SimpleTransaction::Transfer {
                from: alice(),
                to: bob(),
                value: 300,
            }]),
            ExecInputs::new_empty(),
        );
        let blkid2 = block2.get_header().compute_block_id();

        // Block 3: alice -> bob 100
        let (block3, inp3, out3, _state3) = build_block(
            &ee,
            &state2,
            blkid2,
            3,
            SimpleBlockBody::new(vec![SimpleTransaction::Transfer {
                from: alice(),
                to: bob(),
                value: 100,
            }]),
            ExecInputs::new_empty(),
        );

        // Aggregate inputs and outputs across the chunk.
        let chunk_inputs = ExecInputs::new_empty();

        let chunk_outputs = ExecOutputs::new_empty();

        // Build the chunk.
        let chunk_blocks = vec![
            ChunkBlock::new(&inp1, &out1, block1),
            ChunkBlock::new(&inp2, &out2, block2),
            ChunkBlock::new(&inp3, &out3, block3),
        ];
        let chunk = Chunk::new(chunk_blocks);

        // Process starting from the initial state.
        let mut state = initial_state;
        process_chunk_blocks(
            &ee,
            &mut state,
            &chunk,
            genesis_blkid,
            &chunk_inputs,
            &chunk_outputs,
        )
        .expect("multi-block chunk should process successfully");

        // Verify final balances: alice=1000-200-300-100=400, bob=200+300+100=600
        assert_eq!(state.accounts().get(&alice()), Some(&400));
        assert_eq!(state.accounts().get(&bob()), Some(&600));
    }
}
