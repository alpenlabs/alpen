//! Context impl to instantiate chain worker with.

use std::sync::Arc;

use bitcoind_async_client::{client::Client, traits::Reader};
use strata_asm_common::AnchorState;
use strata_asm_worker::{WorkerContext, WorkerResult};
use strata_db::DbError;
use strata_primitives::prelude::*;
use strata_storage::{AsmManager, L1BlockManager};
use tracing::*;

#[expect(missing_debug_implementations)]
pub struct AsmWorkerCtx {
    l1man: Arc<L1BlockManager>,
    bitcoin_client: Arc<Client>,
    asmman: Arc<AsmManager>,
}

impl AsmWorkerCtx {
    pub fn new(
        l1man: Arc<L1BlockManager>,
        bitcoin_client: Arc<Client>,
        asmman: Arc<AsmManager>,
    ) -> Self {
        Self {
            l1man,
            bitcoin_client,
            asmman,
        }
    }
}

impl WorkerContext for AsmWorkerCtx {
    fn get_l1_block(&self, blockid: &L1BlockId) -> WorkerResult<bitcoin::Block> {
        //self.l1man
        //    .get_block_manifest(blockid)
        //    .map_err(conv_db_err)?
        //    .ok_or(strata_asm_worker::WorkerError::NotInitialized)
        todo!()
    }

    fn get_latest_asm_state(&self) -> WorkerResult<Option<(L1BlockCommitment, AnchorState)>> {
        todo!()
    }

    fn get_anchor_state(&self, blockid: &L1BlockCommitment) -> WorkerResult<AnchorState> {
        todo!()
    }

    fn store_anchor_state(
        &self,
        blockid: &L1BlockCommitment,
        state: &AnchorState,
    ) -> WorkerResult<()> {
        todo!()
    }

    fn get_network(&self) -> WorkerResult<bitcoin::Network> {
        todo!()
    }
}

fn conv_db_err(_e: DbError) -> strata_asm_worker::WorkerError {
    // TODO fixme
    strata_asm_worker::WorkerError::Unimplemented
}
