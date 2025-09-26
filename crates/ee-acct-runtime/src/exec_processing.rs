//! Procedures relating more specifically to execution processing.

use strata_acct_types::Hash;
use strata_ee_acct_types::{
    CommitBlockData, CommitChainSegment, CommitMsgData, EeAccountState, EnvError, EnvResult,
    ExecBlockOutput, ExecutionEnvironment,
};
use strata_ee_chain_types::{BlockInputs, ExecBlockNotpackage};

use crate::verification_state::UpdateVerificationState;

struct ChainVerificationState<'v, E: ExecutionEnvironment> {
    uvstate: &'v mut UpdateVerificationState,
    ee: &'v E,

    exec_state: E::PartialState<'v>,
    last_exec_header: E::Header<'v>,
    last_exec_blkid: Hash,
}

impl<'v, E: ExecutionEnvironment> ChainVerificationState<'v, E> {
    /// Computes the state root of the current chain verification state.
    fn compute_state_root(&self) -> EnvResult<Hash> {
        E::compute_state_root(&self.exec_state)
    }

    /// Processes a block on top of the current exec state, producing and output
    /// but not modifying the state.
    fn process_block(
        &self,
        block: &E::Block<'_>,
        inputs: &BlockInputs,
    ) -> EnvResult<ExecBlockOutput<E>> {
        self.ee.process_block(&self.exec_state, block, inputs)
    }

    /// Merges a write batch into the current state.
    fn apply_write_batch(&mut self, wb: &E::WriteBatch) -> EnvResult<()> {
        self.ee.merge_write_into_state(&mut self.exec_state, wb)
    }
}

/// Verifies a chain segment, accumulating effects in the verification state.
pub fn verify_chain_segment<E: ExecutionEnvironment>(
    cvstate: &mut ChainVerificationState<'_, E>,
    commit: &CommitMsgData,
    segment: &CommitChainSegment,
) -> EnvResult<()> {
    // 1. Make sure the last block of this package chain matches the exec blkid
    // that was committed.  This is enough for us, we don't actually care about
    // the notpackage chain beyond this.
    let Some(last) = segment.blocks().last() else {
        return Err(EnvError::MalformedCoinput);
    };

    let last_exec_blkid = last.notpackage().exec_blkid();
    if last_exec_blkid != commit.chunk_commitment() {
        return Err(EnvError::MismatchedCoinput);
    }

    // 2. Go through each block and process them.
    for block_data in segment.blocks() {
        process_block(cvstate, block_data)?;
    }

    Ok(())
}

fn process_block<E: ExecutionEnvironment>(
    cvstate: &mut ChainVerificationState<'_, E>,
    block_data: &CommitBlockData,
) -> EnvResult<()> {
    // 1. Make sure the raw block data matches the package.
    // TODO

    // 2. Decode the block and make sure the actual block ID matches.
    let block = E::decode_block(block_data.raw_full_block())?;
    let header = E::get_block_header(&block);
    let blkid = E::compute_block_id(&header);
    if blkid != block_data.notpackage().exec_blkid() {
        return Err(EnvError::InconsistentCoinput);
    }

    // 2. Execute the block and make sure the outputs are consistent with the
    // package.
    let exec_outp = cvstate.process_block(&block, block_data.notpackage().inputs())?;
    if exec_outp.outputs() != block_data.notpackage().outputs() {
        return Err(EnvError::InconsistentCoinput);
    }

    // 3. Apply writes and check state root.
    cvstate.apply_write_batch(exec_outp.write_batch())?;
    let computed_sr = cvstate.compute_state_root()?;
    let header_sr = E::get_header_state_root(&header);
    if computed_sr != header_sr {
        return Err(EnvError::InconsistentCoinput);
    }

    // 4. maybe: Compare summaries?  (or other DA something something)

    // 5. Apply outputs.
    cvstate
        .uvstate
        .merge_block_outputs(block_data.notpackage().outputs());

    // TODO other stuff?  how do we do fincls?

    Ok(())
}

pub fn apply_commit(astate: &mut EeAccountState, commit: &CommitMsgData) -> EnvResult<()> {
    // 1. The first part is easy, we just update the value.
    astate.set_last_exec_blkid(commit.chunk_commitment());

    // 2. The second part is a little harder, we have to figure out what pending
    // inputs we're consuming from the state so we can remove those.
    // TODO

    Ok(())
}
