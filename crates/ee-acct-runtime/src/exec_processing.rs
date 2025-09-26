//! Procedures relating more specifically to execution processing.

use strata_ee_acct_types::{
    CommitBlockData, CommitCoinput, CommitMsgData, EeAccountState, EnvError, EnvResult,
    ExecutionEnvironment,
};
use strata_ee_chain_types::ExecBlockNotpackage;

use crate::verification_state::UpdateVerificationState;

/// Verifies a commit against the account's current state using a coinput which
/// reveals all the EE blocks and notpackage data, accumulating effects in the
/// verification state.
pub fn verify_commit<E: ExecutionEnvironment>(
    vstate: &mut UpdateVerificationState,
    astate: &EeAccountState,
    commit: &CommitMsgData,
    coinp: &CommitCoinput,
    ee: &E,
) -> EnvResult<()> {
    // 1. Make sure the last block of this package chain matches the exec blkid
    // that was committed.  This is enough for us, we don't actually care about
    // the notpackage chain beyond this.
    let Some(last) = coinp.blocks().last() else {
        return Err(EnvError::MalformedCoinput);
    };

    let last_exec_blkid = last.notpackage().exec_blkid();
    if last_exec_blkid != commit.chunk_commitment() {
        return Err(EnvError::MismatchedCoinput);
    }

    // 2. Decode the previous header and make sure it matches what's in our cur state.
    let prev_header = E::decode_header(coinp.raw_prev_header())?;
    let prev_header_blkid = E::compute_block_id(&prev_header);
    if prev_header_blkid != astate.last_exec_blkid() {
        return Err(EnvError::MismatchedCoinput);
    }

    // 3. Decode the partial state, check it matches the prev header's state root.
    let prev_header_sr = E::get_header_state_root(&prev_header);
    let mut partial_state = E::decode_partial_state(coinp.raw_partial_state())?;
    let ps_sr = E::compute_state_root(&partial_state)?;
    if ps_sr != prev_header_sr {
        return Err(EnvError::MismatchedCoinput);
    }

    // 4. Go through each block and process them.
    for block_data in coinp.blocks() {
        process_block(vstate, &mut partial_state, block_data, ee)?;
    }

    // We don't update the
    Ok(())
}

fn process_block<E: ExecutionEnvironment>(
    vstate: &mut UpdateVerificationState,
    pstate: &mut E::PartialState<'_>,
    block_data: &CommitBlockData,
    ee: &E,
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
    let exec_outp = ee.process_block(pstate, &block, block_data.notpackage().inputs())?;
    if exec_outp.outputs() != block_data.notpackage().outputs() {
        return Err(EnvError::InconsistentCoinput);
    }

    // 3. Apply writes and check state root.
    ee.merge_write_into_state(pstate, exec_outp.write_batch())?;
    let computed_sr = E::compute_state_root(pstate)?;
    let header_sr = E::get_header_state_root(&header);
    if computed_sr != header_sr {
        return Err(EnvError::InconsistentCoinput);
    }

    // 4. maybe: Compare summaries?

    // 5. Apply writes.
    // TODO

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
