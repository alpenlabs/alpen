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
use strata_ee_chain_types::{BlockInputs, ExecBlockNotpackage, SubjectDepositData};

use crate::verification_state::UpdateVerificationState;

struct InputTracker<'a> {
    expected_inputs: &'a [PendingInputEntry],
    consumed: usize,
}

impl<'a> InputTracker<'a> {
    pub fn new(expected_inputs: &'a [PendingInputEntry]) -> Self {
        Self {
            expected_inputs,
            consumed: 0,
        }
    }

    fn consumed(&self) -> usize {
        self.consumed
    }

    fn has_next(&self) -> bool {
        self.consumed < self.expected_inputs.len()
    }

    fn expected_next(&self) -> Option<&'a PendingInputEntry> {
        if self.has_next() {
            Some(&self.expected_inputs[self.consumed])
        } else {
            None
        }
    }

    /// Checks if an input matches the next value we expect to consume.  If it
    /// matches, increments the pointer.  Errors on mismatch.
    pub fn consume_input(&mut self, input: &PendingInputEntry) -> EnvResult<()> {
        let Some(exp_next) = self.expected_next() else {
            return Err(EnvError::MalformedCoinput);
        };

        if input != exp_next {
            return Err(EnvError::MalformedCoinput);
        }

        Ok(())
    }
}

struct ChainVerificationState<'v, E: ExecutionEnvironment> {
    uvstate: &'v mut UpdateVerificationState,
    ee: &'v E,

    input_tracker: InputTracker<'v>,

    exec_state: E::PartialState,
    last_exec_header: <E::Block as ExecBlock>::Header,
    last_exec_blkid: Hash,
}

impl<'v, E: ExecutionEnvironment> ChainVerificationState<'v, E> {
    /// Computes the state root of the current chain verification state.
    fn compute_cur_state_root(&self) -> EnvResult<Hash> {
        self.exec_state.compute_state_root()
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
}

/// Verifies a chain segment, accumulating effects in the verification state.
pub fn verify_chain_segment<E: ExecutionEnvironment>(
    cvstate: &mut ChainVerificationState<'_, E>,
    commit: &CommitMsgData,
    segment: &CommitChainSegment,
    extra: &UpdateExtraData,
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
    let raw_block_hash = Sha256::digest(block_data.raw_full_block());
    if raw_block_hash.as_ref() != block_data.notpackage().raw_block_encoded_hash() {
        return Err(EnvError::InconsistentCoinput);
    }

    // 2. Decode the block and make sure the actual block ID matches.
    let block: E::Block =
        decode_buf_exact(block_data.raw_full_block()).map_err(|_| EnvError::Decode)?;
    let header = block.get_header();
    let blkid = header.compute_block_id();
    if blkid != block_data.notpackage().exec_blkid() {
        return Err(EnvError::InconsistentCoinput);
    }

    // 3. Check that the inputs match what was expected.
    for inp in block_data.notpackage().inputs().subject_deposits() {
        // We kinda bodge this conversion for now since the concepts aren't a
        // 1:1 match, maybe this will be iterated on in the future if we rethink
        // the types.  But this is fine for now.
        let pi = PendingInputEntry::Deposit(SubjectDepositData::new(inp.dest(), inp.value()));
        cvstate.input_tracker.consume_input(&pi)?;
    }

    // 4. Execute the block and make sure the outputs are consistent with the
    // package.
    let exec_outp = cvstate.process_block(&block, block_data.notpackage().inputs())?;
    if exec_outp.outputs() != block_data.notpackage().outputs() {
        return Err(EnvError::InconsistentCoinput);
    }

    // 5. Apply writes and check state root.  This checks the state root
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

pub fn apply_commit(
    astate: &mut EeAccountState,
    commit: &CommitMsgData,
    extra: &UpdateExtraData,
) -> EnvResult<()> {
    // 1. The first part is easy, we just update the value.
    astate.set_last_exec_blkid(commit.chunk_commitment());

    // 2. The second part is a little harder, we have to figure out what pending
    // inputs we're consuming from the state so we can remove those.
    astate.remove_pending_inputs(extra.processed_inputs() as usize);
    astate.remove_pending_fincls(extra.processed_fincls() as usize);

    // TODO update tracked balance?  this involves a little more indirect reasoning

    Ok(())
}
