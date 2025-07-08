//! Generic exec worker task.

use std::sync::Arc;

use strata_common::retry::{
    policies::ExponentialBackoff, retry_with_backoff, DEFAULT_ENGINE_CALL_MAX_RETRIES,
};
use strata_primitives::l2::L2BlockCommitment;
use strata_storage::L2BlockManager;
use strata_tasks::ShutdownGuard;
use tracing::{debug, error, info, warn};

use crate::{
    engine::*,
    errors::{EngineError, EngineResult},
    handle::{ExecCommand, ExecCtlInput},
    messages::ExecPayloadData,
    sync::sync_chainstate_to_el,
};

#[expect(missing_debug_implementations)]
pub struct ExecWorkerState<E: ExecEngineCtl> {
    engine: Arc<E>,

    exec_env_id: ExecEnvId,

    safe_tip: L2BlockCommitment,
    finalized_tip: L2BlockCommitment,
    l2_storage: Arc<L2BlockManager>,
}

impl<E: ExecEngineCtl> ExecWorkerState<E> {
    /// Constructs a new instance.
    pub fn new(
        engine: Arc<E>,
        exec_env_id: ExecEnvId,
        safe_tip: L2BlockCommitment,
        finalized_tip: L2BlockCommitment,
        l2_storage: Arc<L2BlockManager>,
    ) -> Self {
        Self {
            engine,
            exec_env_id,
            safe_tip,
            finalized_tip,
            l2_storage,
        }
    }

    /// Make a call to the exec engine, using retry and backoff.
    fn call_engine<T>(&mut self, name: &str, f: impl Fn(&E) -> EngineResult<T>) -> EngineResult<T> {
        let res = retry_with_backoff(
            name,
            DEFAULT_ENGINE_CALL_MAX_RETRIES,
            &ExponentialBackoff::default(),
            move || f(&self.engine),
        )?;
        Ok(res)
    }

    fn check_tip_block_exists(&mut self) -> EngineResult<bool> {
        let blkid = *self.safe_tip.blkid();
        self.call_engine("engine_check_block_exists", |eng| {
            eng.check_block_exists(L2BlockRef::Id(blkid))
        })
    }

    fn update_safe_tip(&mut self, new_safe: &L2BlockCommitment) -> EngineResult<()> {
        self.safe_tip = *new_safe;
        self.call_engine("engine_update_safe_tip", |eng| {
            eng.update_safe_block(*new_safe.blkid())?;
            Ok(())
        })
    }

    fn update_finalized_tip(&mut self, new_finalized: &L2BlockCommitment) -> EngineResult<()> {
        self.call_engine("engine_update_finalized_tip", |eng| {
            eng.update_finalized_block(*new_finalized.blkid())?;
            Ok(())
        })
    }

    /// Calls the engine to update the reffed blocks.
    fn update_engine_refs(&mut self) -> EngineResult<()> {
        let safe_blkid = *self.safe_tip.blkid();
        let finalized_blkid = *self.finalized_tip.blkid();
        self.call_engine("engine_update_refs", |eng| {
            eng.update_safe_block(safe_blkid)?;
            eng.update_finalized_block(finalized_blkid)?;
            Ok(())
        })
    }

    /// Tries to exec an EL payload.
    fn try_exec_el_payload(
        &mut self,
        blkid: &L2BlockCommitment,
        payload: &ExecPayloadData,
    ) -> EngineResult<()> {
        // We don't do this for the genesis block because that block doesn't
        // actually have a well-formed accessory and it gets mad at us.
        if blkid.slot() == 0 {
            return Ok(());
        }

        // Construct the exec payload and just make the call.  This blocks until
        // it gets back to us, which kinda sucks, but we're working on it!
        //
        // TODO this needs to be refactored since we might not always be able to
        // get this data from the block itself
        // let _exec_hash = bundle.header().exec_payload_hash();
        // let eng_payload = ExecPayloadData::from_l2_block_bundle(bundle);
        let res = self.call_engine("engine_submit_payload", move |eng| {
            // annoying that we're cloning this each time, maybe make it take a ref?
            eng.submit_payload(payload.clone())
        })?;

        if res == BlockStatus::Invalid {
            Err(EngineError::InvalidPayload(*blkid))
        } else {
            Ok(())
        }
    }
}

/// Execution controller worker task entrypoint.
pub fn exec_worker_task<E: ExecEngineCtl>(
    shutdown: ShutdownGuard,
    mut state: ExecWorkerState<E>,
    mut input: ExecCtlInput,
    context: &impl ExecWorkerContext,
) -> anyhow::Result<()> {
    info!("starting exec worker");

    // Check that tip L2 block exists (and engine can be connected to)
    let chain_tip = &state.safe_tip.clone();
    match state.check_tip_block_exists() {
        Ok(true) => {
            info!("startup: last l2 block is synced")
        }
        Ok(false) => {
            // Current chain tip tip block is not known by the EL.
            warn!(?chain_tip, "missing expected EVM block");
            sync_chainstate_to_el(&state.l2_storage, state.engine.as_ref())?;
        }
        Err(error) => {
            // Likely network issue
            anyhow::bail!("could not connect to exec engine, err = {}", error);
        }
    }

    while let Some(inp) = input.recv_msg() {
        match inp {
            ExecCommand::NewBlock(block, completion) => {
                debug!("new block here");
                let payload = context.fetch_exec_payload(&block, &state.exec_env_id)?;
                // TODO figure out how to call the engine with the payload we got
                match payload {
                    Some(payload) => {
                        let res = state.try_exec_el_payload(&block, &payload);
                        match res {
                            Ok(()) => info!("Executed EL payload"),
                            Err(e) => error!(%e, "Error in executing EL payload"),
                        }
                    }
                    None => {
                        warn!("No payload");
                    }
                }
                let _ = completion.send(Ok(()));
            }
            ExecCommand::NewSafeTip(ts, completion) => {
                let res = state.update_safe_tip(&ts);
                let _ = completion.send(res);
            }
            ExecCommand::NewFinalizedTip(ts, completion) => {
                let res = state.update_safe_tip(&ts);
                let _ = completion.send(res);
            }
        }
        if shutdown.should_shutdown() {
            break;
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
