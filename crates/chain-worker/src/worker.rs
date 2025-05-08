//! Chain executor worker task.

use strata_primitives::prelude::*;

use crate::{
    WorkerError, WorkerResult,
    handle::{ChainWorkerInput, WorkerMessage},
};

pub struct WorkerState {
    // TODO
}

pub fn worker_task(mut state: WorkerState, mut input: ChainWorkerInput) -> anyhow::Result<()> {
    while let Some(m) = input.recv_next() {
        match m {
            WorkerMessage::TryExecBlock(l2bc, completion) => {
                let res = do_try_exec_block(&mut state, l2bc);
                let _ = completion.send(res);
            }
        }
    }

    Ok(())
}

fn do_try_exec_block(state: &mut WorkerState, l2bc: L2BlockCommitment) -> WorkerResult<()> {
    // TODO call out to chain executor, update database
    // TODO call out to exec engine controller
    Err(WorkerError::Unimplemented)
}
