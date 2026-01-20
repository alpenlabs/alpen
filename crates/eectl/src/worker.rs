//! Generic exec worker task.

use std::{fmt::Debug, sync::Arc};

use strata_common::retry::{
    policies::ExponentialBackoff, retry_with_backoff, DEFAULT_ENGINE_CALL_MAX_RETRIES,
};
use strata_db_types::DbError;
use strata_ol_chain_types::L2BlockId;
use strata_primitives::{epoch::EpochCommitment, l2::L2BlockCommitment};
use strata_status::StatusChannel;
use strata_tasks::ShutdownGuard;
use tokio::{runtime::Handle, time};
use tracing::{debug, error, info, warn};

use crate::{
    engine::*,
    errors::{EngineError, EngineResult},
    handle::{ExecCommand, ExecCtlInput},
    messages::ExecPayloadData,
};

#[expect(
    missing_debug_implementations,
    reason = "some inner types don't have Debug impls"
)]
pub struct ExecWorkerState<E: ExecEngineCtl> {
    engine: Arc<E>,
    exec_env_id: ExecEnvId,
    safe_tip: L2BlockCommitment,
    _finalized_tip: L2BlockCommitment,
}

impl<E: ExecEngineCtl> ExecWorkerState<E> {
    /// Constructs a new instance.
    pub fn new(
        engine: Arc<E>,
        exec_env_id: ExecEnvId,
        safe_tip: L2BlockCommitment,
        finalized_tip: L2BlockCommitment,
    ) -> Self {
        Self {
            engine,
            exec_env_id,
            safe_tip,
            _finalized_tip: finalized_tip,
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

    #[expect(unused, reason = "will be used later")]
    fn update_finalized_tip(&mut self, new_finalized: &L2BlockCommitment) -> EngineResult<()> {
        self.call_engine("engine_update_finalized_tip", |eng| {
            eng.update_finalized_block(*new_finalized.blkid())?;
            Ok(())
        })
    }

    /// Calls the engine to update the reffed blocks.
    #[expect(unused, reason = "will be used later")]
    fn update_engine_refs(&mut self) -> EngineResult<()> {
        let safe_blkid = *self.safe_tip.blkid();
        let finalized_blkid = *self._finalized_tip.blkid();
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

    /// Sync missing blocks in EL using payloads stored in L2 block database.
    ///
    /// TODO: retry on network errors
    pub fn sync_missing_blocks_to_el(
        &mut self,
        context: &impl ExecWorkerContext,
    ) -> Result<(), EngineError> {
        info!("Syncing chainstate to EL");
        let tip_block = context.fetch_cur_tip()?;
        debug!(?tip_block, "L2 tip block");

        let tip_idx = tip_block.slot();

        // last idx of chainstate whose corresponding block is present in el
        let sync_from_idx = find_last_match((0, tip_idx), |idx| {
            let blkid = context
                .fetch_blkid_at_height(idx)?
                .ok_or(DbError::MissingL2BlockHeight(idx))?;
            self.engine.check_block_exists(L2BlockRef::Id(blkid))
        })?
        .map(|idx| idx + 1) // sync from next index
        .unwrap_or(0); // sync from genesis
        info!(%sync_from_idx, "last known EL block index");

        if sync_from_idx >= tip_idx {
            info!("EL in sync with chainstate");
            return Ok(());
        }

        // Collect all payloads from sync_from_idx..=tip_idx
        let mut payloads_to_sync = Vec::with_capacity((tip_idx - sync_from_idx) as usize + 1);
        let mut block_to_sync = tip_block;
        for _ in sync_from_idx..=tip_idx {
            let payload = context
                .fetch_exec_payload(&block_to_sync, &self.exec_env_id)?
                .ok_or(DbError::MissingL2Block(*block_to_sync.blkid()))?;
            payloads_to_sync.push((block_to_sync, payload));
            block_to_sync = context.fetch_parent(&block_to_sync)?;
        }
        payloads_to_sync.reverse();

        // Sanity check
        if let (Some((first_block, _)), Some((last_block, _))) =
            (payloads_to_sync.first(), payloads_to_sync.last())
        {
            assert_eq!(first_block.slot(), sync_from_idx);
            assert_eq!(last_block.slot(), tip_idx);
        }

        for (block, payload) in payloads_to_sync {
            debug!(?payload, "Submitting payload to engine");
            self.call_engine("engine_submit_payload", |eng| {
                eng.submit_payload(payload.clone())
            })?;
            self.call_engine("engine_update_safe_block", |eng| {
                eng.update_safe_block(*block.blkid())
            })?;
        }

        Ok(())
    }
}

fn find_last_match(
    range: (u64, u64),
    predicate: impl Fn(u64) -> Result<bool, EngineError>,
) -> Result<Option<u64>, EngineError> {
    let (mut left, mut right) = range;

    // Check the leftmost value first
    if !predicate(left)? {
        return Ok(None); // If the leftmost value is false, no values can be true
    }

    let mut best_match = None;

    // Proceed with binary search
    while left <= right {
        let mid = left + (right - left) / 2;

        if predicate(mid)? {
            best_match = Some(mid); // Update best match
            left = mid + 1; // Continue searching in the right half
        } else {
            right = mid - 1; // Search in the left half
        }
    }

    Ok(best_match)
}

/// Execution controller worker task entrypoint.
pub fn worker_task_inner<E: ExecEngineCtl>(
    shutdown: ShutdownGuard,
    mut state: ExecWorkerState<E>,
    mut input: ExecCtlInput,
    context: &impl ExecWorkerContext,
) -> anyhow::Result<()> {
    // Check that tip L2 block exists (and engine can be connected to)
    let chain_tip = &state.safe_tip.clone();
    match state.check_tip_block_exists() {
        Ok(true) => {
            info!("startup: last l2 block is synced")
        }
        Ok(false) => {
            // Current chain tip tip block is not known by the EL.
            warn!(?chain_tip, "missing expected EVM block");
            state.sync_missing_blocks_to_el(context)?;
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

pub(crate) fn worker_task<
    E: ExecEngineCtl + Sync + Send + 'static,
    State: Clone + Debug + Send + Sync + 'static,
>(
    shutdown: ShutdownGuard,
    handle: Handle,
    context: &impl ExecWorkerContext,
    _status_channel: StatusChannel<State>,
    engine: Arc<E>,
    exec_rx: ExecCtlInput,
) -> anyhow::Result<()> {
    info!("waiting until genesis");

    // TODO(QQ): maybe expose better waiting for L2 genesis through status channel.
    let genesis_block_id = handle.block_on(async {
        while context.fetch_blkid_at_height(0).unwrap().is_none() {
            time::sleep(time::Duration::from_secs(1)).await;
        }
        context
            .fetch_blkid_at_height(0)
            .unwrap()
            .expect("genesis should happen")
    });

    let init_state = context.fetch_latest_finalized_epoch()?;

    let finalized_tip = match init_state {
        Some(epoch) => epoch.to_block_commitment(),
        None => L2BlockCommitment::new(0, genesis_block_id),
    };

    let cur_tip = context.fetch_cur_tip()?;

    info!(?cur_tip, ?finalized_tip, "starting exec worker");

    let exec_env_id = ();
    let state = ExecWorkerState::new(engine, exec_env_id, cur_tip, finalized_tip);
    worker_task_inner(shutdown, state, exec_rx, context)?;
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

    /// Retrieves the parent block commitment, or returns an error if unable to fetch the parent
    fn fetch_parent(&self, block: &L2BlockCommitment) -> EngineResult<L2BlockCommitment>;

    /// Retrieves the current tip, or returns an error if unable to fetch the tip.
    fn fetch_cur_tip(&self) -> EngineResult<L2BlockCommitment>;

    /// Retrieves block ID at height, returning `None` if the height is valid but the block doesn't
    /// exist.
    fn fetch_blkid_at_height(&self, height: u64) -> EngineResult<Option<L2BlockId>>;

    fn fetch_latest_finalized_epoch(&self) -> EngineResult<Option<EpochCommitment>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_last_match() {
        // find match
        assert!(matches!(
            find_last_match((0, 5), |idx| Ok(idx < 3)),
            Ok(Some(2))
        ));
        // found no match
        assert!(matches!(find_last_match((0, 5), |_| Ok(false)), Ok(None)));
        // got error
        let error_message = "intentional error for test";
        assert!(matches!(
            find_last_match((0, 5), |_| Err(EngineError::Other(error_message.into()))),
            Err(err) if err.to_string().contains(error_message)
        ));
    }
}
