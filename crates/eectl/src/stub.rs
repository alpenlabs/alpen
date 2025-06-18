//! Stub engine controller that we use for testing without having to plug in a
//! full EVM runtime.
//!
//! This just simulates producing a payload by waiting some amount before
//! returning `Ready` with dummy state.  We might extend this slightly to make
//! it more believable.

use std::{collections::*, sync::Mutex, time};

use strata_primitives::buf::Buf32;
use strata_state::{
    exec_update::{ExecUpdate, UpdateInput, UpdateOutput},
    prelude::*,
};

use crate::{engine::*, errors::*, messages::*};

struct State {
    next_idx: u64,
    payload_jobs: HashMap<u64, time::Instant>,
}

impl State {
    fn new() -> Self {
        Self {
            next_idx: 1,
            payload_jobs: HashMap::new(),
        }
    }
}

pub struct StubController {
    payload_prep_dur: time::Duration,
    state: Mutex<State>,
}

impl StubController {
    pub fn new(payload_prep_dur: time::Duration) -> Self {
        Self {
            payload_prep_dur,
            state: Mutex::new(State::new()),
        }
    }
}

impl ExecEngineCtl for StubController {
    fn submit_payload(&self, _payload: ExecPayloadData) -> EngineResult<BlockStatus> {
        Ok(BlockStatus::Valid)
    }

    fn prepare_payload(&self, _env: PayloadEnv) -> EngineResult<u64> {
        // TODO do something with the payloads to make the status more believable
        let mut state = self.state.lock().unwrap();
        let idx = state.next_idx;
        state.next_idx += 1;
        state.payload_jobs.insert(idx, time::Instant::now());
        Ok(idx)
    }

    fn get_payload_status(&self, id: u64) -> EngineResult<PayloadStatus> {
        let state = self.state.lock().unwrap();
        let created_at = state
            .payload_jobs
            .get(&id)
            .ok_or(EngineError::UnknownPayloadId(id))?;

        let now = time::Instant::now();
        if *created_at + self.payload_prep_dur > now {
            Ok(PayloadStatus::Working)
        } else {
            // TODO make up a more plausible payload
            let exec = ExecPayloadData::new(
                ExecUpdate::new(
                    UpdateInput::new(0, vec![], Buf32::zero(), Vec::new()),
                    UpdateOutput::new_from_state(Buf32::zero()),
                ),
                Vec::new(),
                Vec::new(),
            );
            Ok(PayloadStatus::Ready(exec, 0))
        }
    }

    fn update_head_block(&self, _id: L2BlockId) -> EngineResult<()> {
        Ok(())
    }

    fn update_safe_block(&self, _id: L2BlockId) -> EngineResult<()> {
        Ok(())
    }

    fn update_finalized_block(&self, _id: L2BlockId) -> EngineResult<()> {
        Ok(())
    }

    fn check_block_exists(&self, _ref: L2BlockRef<'_>) -> EngineResult<bool> {
        Ok(true)
    }
}
