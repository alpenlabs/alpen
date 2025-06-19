//! Generic exec worker task.

use strata_common::retry::{
    policies::ExponentialBackoff, retry_with_backoff, DEFAULT_ENGINE_CALL_MAX_RETRIES,
};
use strata_primitives::{
    epoch::EpochCommitment,
    l2::{L2BlockCommitment, L2BlockId},
};
use strata_state::{block::L2BlockBundle, header::L2Header};

use crate::{
    engine::*,
    errors::{EngineError, EngineResult},
    handle::{ExecCommand, ExecCtlInput},
    messages::{ExecPayloadData, TipState},
};

pub struct WorkerState<E: ExecEngineCtl> {
    engine: E,

    exec_env_id: ExecEnvId,

    cur_tip: L2BlockCommitment,
    prev_epoch: EpochCommitment,
}

impl<E: ExecEngineCtl> WorkerState<E> {
    /// Constructs a new instance.
    pub fn new(
        engine: E,
        exec_env_id: ExecEnvId,
        cur_tip: L2BlockCommitment,
        prev_epoch: EpochCommitment,
    ) -> Self {
        Self {
            engine,
            exec_env_id,
            cur_tip,
            prev_epoch,
        }
    }

    /// Make a call to the exec engine, using retry and backoff.
    fn call_engine<T>(
        &mut self,
        name: &str,
        f: impl Fn(&mut E) -> EngineResult<T>,
    ) -> EngineResult<T> {
        let res = retry_with_backoff(
            name,
            DEFAULT_ENGINE_CALL_MAX_RETRIES,
            &ExponentialBackoff::default(),
            move || f(&mut self.engine),
        )?;
        Ok(res)
    }

    /// Calls the engine to update the reffed blocks.
    fn update_engine_refs(&mut self) -> EngineResult<()> {
        let safe = *self.cur_tip.blkid();
        let finalized = *self.prev_epoch.last_blkid();
        self.call_engine("engine_update_refs", |eng| {
            eng.update_safe_block(safe)?;
            eng.update_finalized_block(finalized)?;
            Ok(())
        })
    }

    /// Updates the tip state and updates the underlying engine's refs.
    fn update_tip_state(&mut self, new_tip_state: &TipState) -> EngineResult<()> {
        self.cur_tip = *new_tip_state.cur_tip();
        self.prev_epoch = *new_tip_state.prev_epoch();

        self.update_engine_refs()?;
        Ok(())
    }

    /// Tries to exec an EL payload.
    fn try_exec_el_payload(
        &mut self,
        blkid: &L2BlockId,
        bundle: &L2BlockBundle,
    ) -> EngineResult<()> {
        // We don't do this for the genesis block because that block doesn't
        // actually have a well-formed accessory and it gets mad at us.
        if bundle.header().slot() == 0 {
            return Ok(());
        }

        // Construct the exec payload and just make the call.  This blocks until
        // it gets back to us, which kinda sucks, but we're working on it!
        //
        // TODO this needs to be refactored since we might not always be able to
        // get this data from the block itself
        let exec_hash = bundle.header().exec_payload_hash();
        let eng_payload = ExecPayloadData::from_l2_block_bundle(bundle);
        let res = self.call_engine("engine_submit_payload", move |eng| {
            // annoying that we're cloning this each time, maybe make it take a ref?
            eng.submit_payload(eng_payload.clone())
        })?;

        if res == BlockStatus::Invalid {
            let block = L2BlockCommitment::new(bundle.header().slot(), *blkid);
            Err(EngineError::InvalidPayload(block).into())
        } else {
            Ok(())
        }
    }
}

/// Execution controller worker task entrypoint.
pub fn worker_task<E: ExecEngineCtl>(
    mut state: WorkerState<E>,
    mut input: ExecCtlInput,
    context: &impl ExecWorkerContext,
) -> anyhow::Result<()> {
    while let Some(inp) = input.recv_msg() {
        match inp {
            ExecCommand::NewTipState(ts, completion) => {
                let res = state.update_tip_state(&ts);
                let _ = completion.send(res);
            }

            ExecCommand::NewBlock(block, completion) => {
                let payload = context.fetch_exec_payload(&block, &state.exec_env_id)?;
                // TODO figure out how to call the engine with the payload we got
                let _ = completion.send(Ok(()));
            }
        }
    }

    Ok(())
}

/// ID of the execution env we're watching.
// TODO make this be an account ID or something
pub type ExecEnvId = ();

/// Context for exec worker.
pub trait ExecWorkerContext {
    /// Fetches the new exec payload for a block, if there is one.
    fn fetch_exec_payload(
        &self,
        block: &L2BlockCommitment,
        eeid: &ExecEnvId,
    ) -> EngineResult<Option<ExecPayloadData>>;
}
