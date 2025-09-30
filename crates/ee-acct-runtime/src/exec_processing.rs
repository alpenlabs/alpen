//! Procedures relating more specifically to execution processing.

use digest::Digest;
use sha2::Sha256;
use strata_acct_types::Hash;
use strata_codec::decode_buf_exact;
use strata_ee_acct_types::{
    CommitBlockData, CommitChainSegment, CommitMsgData, EeAccountState, EnvError, EnvResult,
    ExecBlock, ExecBlockOutput, ExecHeader, ExecPartialState, ExecutionEnvironment,
    PendingInputEntry, UpdateExtraData,
};
use strata_ee_chain_types::{BlockInputs, SubjectDepositData};

use crate::verification_state::{InputTracker, PendingCommit, UpdateVerificationState};

struct ChainVerificationState<'v, 'a, E: ExecutionEnvironment> {
    uvstate: &'v mut UpdateVerificationState,
    input_tracker: &'v mut InputTracker<'a>,

    ee: &'v E,

    exec_state: E::PartialState,
    last_exec_header: <E::Block as ExecBlock>::Header,
    last_exec_blkid: Hash,

    processed_commits: usize,
}

impl<'v, 'a, E: ExecutionEnvironment> ChainVerificationState<'v, 'a, E> {
    fn new(
        uvstate: &'v mut UpdateVerificationState,
        input_tracker: &'v mut InputTracker<'a>,
        ee: &'v E,
        exec_state: E::PartialState,
        last_exec_header: <E::Block as ExecBlock>::Header,
    ) -> Self {
        let last_exec_blkid = last_exec_header.compute_block_id();
        Self {
            uvstate,
            input_tracker,
            ee,
            exec_state,
            last_exec_header,
            last_exec_blkid,
            processed_commits: 0,
        }
    }

    /// Computes the state root of the current chain verification state.
    fn compute_cur_state_root(&self) -> EnvResult<Hash> {
        self.exec_state.compute_state_root()
    }

    /// Gets the next commit we have yet to process.
    fn next_pending_commit(&self) -> Option<&PendingCommit> {
        self.uvstate.pending_commits().get(self.processed_commits)
    }

    fn consume_pending_commit(&mut self, exec_blkid: &Hash) -> EnvResult<()> {
        let next_commit = self
            .next_pending_commit()
            .ok_or(EnvError::UncommittedChainSegment)?;

        if *exec_blkid == next_commit.new_tip_exec_blkid() {
            self.processed_commits += 1;
            Ok(())
        } else {
            Err(EnvError::MismatchedChainSegment)
        }
    }

    /// Processes a block on top of the current exec state, producing an output
    /// but not modifying the state.
    fn process_block(
        &self,
        block: &E::Block,
        inputs: &BlockInputs,
    ) -> EnvResult<ExecBlockOutput<E>> {
        self.ee.process_block(&self.exec_state, block, inputs)
    }

    /// Checks that the next pending input entry is this one, and marks it as
    /// consumed.
    fn consume_pending_input(&mut self, input: &PendingInputEntry) -> EnvResult<()> {
        self.input_tracker.consume_input(input)?;
        self.uvstate.inc_consumed_inputs(1);
        Ok(())
    }

    /// Merges a write batch into the current state, also accepting a
    /// corresponding header to check the newly-computed state root against.
    ///
    /// This does NOT check that the blkid matches the header.
    fn apply_write_batch(
        &mut self,
        wb: &E::WriteBatch,
        h: <E::Block as ExecBlock>::Header,
        blkid: Hash,
    ) -> EnvResult<()> {
        self.ee.merge_write_into_state(&mut self.exec_state, wb)?;
        let new_sr = self.compute_cur_state_root()?;

        if new_sr != h.get_state_root() {
            return Err(EnvError::InconsistentCoinput);
        }

        // Okay actually I lied, we *do* check this, but only in tests.  It's
        // expensive to do in the proof so we only want to do it once, but it's
        // fine to do it here as a sanity check.
        #[cfg(test)]
        {
            let computed_blkid = h.compute_block_id();
            assert_eq!(computed_blkid, blkid, "exec: header blkid mismatch");
        }

        self.last_exec_header = h;
        self.last_exec_blkid = blkid;

        Ok(())
    }

    /// Final checks to see if there's anything in the verification state that
    /// were supposed to have been dealt with but weren't.
    fn check_obligations(&self) -> EnvResult<()> {
        if self.next_pending_commit().is_some() {
            return Err(EnvError::UnsatisfiedObligations("pending_commits"));
        }

        Ok(())
    }
}

/// Processes segments against accumulated update verification state by
/// verifying the blocks and managing inputs/outputs/etc.
pub fn process_segments<E: ExecutionEnvironment>(
    uvstate: &mut UpdateVerificationState,
    input_tracker: &mut InputTracker<'_>,
    segments: &[CommitChainSegment],
    pre_state: &[u8],
    cur_tip_header: &[u8],
    ee: &E,
) -> EnvResult<()> {
    // 1. Decode the various inputs to be able to construct the chain
    // verification state tracker thing.
    // TODO maybe use more precise errors here
    let header = decode_buf_exact::<<E::Block as ExecBlock>::Header>(cur_tip_header)
        .map_err(|_| EnvError::MismatchedCurStateData)?;
    let partial_pre_state = decode_buf_exact::<E::PartialState>(pre_state)
        .map_err(|_| EnvError::MismatchedCurStateData)?;

    let pre_state_root = partial_pre_state.compute_state_root()?;
    if pre_state_root != header.get_state_root() {
        return Err(EnvError::MismatchedCurStateData);
    }

    let mut cvstate =
        ChainVerificationState::new(uvstate, input_tracker, ee, partial_pre_state, header);

    // 2. Process each segment in order, continually modifying the chain.
    for segment in segments {
        process_chain_segment(&mut cvstate, segment)?;
    }

    // 3. Final checks.
    cvstate.check_obligations()?;

    Ok(())
}

/// Processes a chain segment by verifying its blocks and accumulating effects
/// in the verification state.
fn process_chain_segment<E: ExecutionEnvironment>(
    cvstate: &mut ChainVerificationState<'_, '_, E>,
    segment: &CommitChainSegment,
) -> EnvResult<()> {
    // 1. Make sure the last block of this package chain matches the exec blkid
    // in the next pending commit.  This is enough for us, we don't actually
    // have a way to directly check the rest of the chain beyond this, but
    // whatever matters will get checked via the EE block processing.
    let Some(last) = segment.blocks().last() else {
        return Err(EnvError::MalformedCoinput);
    };

    let last_exec_blkid = last.notpackage().exec_blkid();
    cvstate.consume_pending_commit(&last_exec_blkid)?;

    // 2. Go through each block and process them.
    for block_data in segment.blocks() {
        process_block(cvstate, block_data)?;
    }

    Ok(())
}

/// Processes a single block.
fn process_block<E: ExecutionEnvironment>(
    cvstate: &mut ChainVerificationState<'_, '_, E>,
    block_data: &CommitBlockData,
) -> EnvResult<()> {
    // 1. Decode the block and make sure the actual block ID matches.
    let block: E::Block = decode_and_check_commit_block::<E>(block_data)?;
    let header = block.get_header();
    let blkid = header.compute_block_id();
    if blkid != block_data.notpackage().exec_blkid() {
        return Err(EnvError::InconsistentCoinput);
    }

    // 2. Execute the block and make sure the outputs are consistent with the
    // package.
    let exec_outp = cvstate.process_block(&block, block_data.notpackage().inputs())?;
    if exec_outp.outputs() != block_data.notpackage().outputs() {
        return Err(EnvError::InconsistentCoinput);
    }

    // 3. Check that the inputs match what was expected.
    for inp in block_data.notpackage().inputs().subject_deposits() {
        // We kinda bodge this conversion for now since the concepts aren't a
        // 1:1 match, maybe this will be iterated on in the future if we rethink
        // the types.  But this is fine for now.
        let pi = PendingInputEntry::Deposit(SubjectDepositData::new(inp.dest(), inp.value()));
        cvstate.consume_pending_input(&pi)?;
    }

    // 4. Apply writes and check state root.  This checks the state root
    // matches the root expected in the header.
    cvstate.apply_write_batch(exec_outp.write_batch(), header.clone(), blkid)?;

    // maybe: Compare summaries?  (or other DA something something)

    // 6. Update bookkeeping outputs.
    cvstate
        .uvstate
        .merge_block_outputs(block_data.notpackage().outputs());

    // TODO other stuff?  how do we do fincls?

    Ok(())
}

/// Checks the raw exec block data from a commit block matches the hash in the
/// notpackage, and then returns the decoded exec block if it matches.
fn decode_and_check_commit_block<E: ExecutionEnvironment>(
    block_data: &CommitBlockData,
) -> EnvResult<E::Block> {
    let raw_block_hash = Sha256::digest(block_data.raw_full_block());
    if raw_block_hash.as_ref() != block_data.notpackage().raw_block_encoded_hash() {
        return Err(EnvError::InconsistentCoinput);
    }

    let block = decode_buf_exact(block_data.raw_full_block())
        .map_err(|_| EnvError::MalformedChainSegment)?;

    Ok(block)
}

pub fn apply_commit(
    astate: &mut EeAccountState,
    commit: &CommitMsgData,
    extra: &UpdateExtraData,
) -> EnvResult<()> {
    // 1. The first part is easy, we just update the value.
    astate.set_last_exec_blkid(*commit.new_tip_exec_blkid());

    // 2. The second part is a little harder, we have to figure out what pending
    // inputs we're consuming from the state so we can remove those.
    astate.remove_pending_inputs(*extra.processed_inputs() as usize);
    astate.remove_pending_fincls(*extra.processed_fincls() as usize);

    // TODO update tracked balance?  this involves a little more indirect reasoning

    Ok(())
}
