//! Context impl to instantiate ASM worker with.

use std::sync::Arc;

use bitcoind_async_client::{client::Client, traits::Reader};
use strata_asm_worker::{WorkerContext, WorkerError, WorkerResult};
use strata_db::DbError;
use strata_primitives::prelude::*;
use strata_state::asm_state::AsmState;
use strata_storage::{AsmStateManager, L1BlockManager};
use tokio::runtime::Handle;

#[expect(
    missing_debug_implementations,
    reason = "Inner types don't have Debug implementation"
)]
pub struct AsmWorkerCtx {
    handle: Handle,
    bitcoin_client: Arc<Client>,
    l1man: Arc<L1BlockManager>,
    asmman: Arc<AsmStateManager>,
}

impl AsmWorkerCtx {
    pub fn new(
        handle: Handle,
        bitcoin_client: Arc<Client>,
        l1man: Arc<L1BlockManager>,
        asmman: Arc<AsmStateManager>,
    ) -> Self {
        Self {
            handle,
            bitcoin_client,
            l1man,
            asmman,
        }
    }
}

impl WorkerContext for AsmWorkerCtx {
    fn get_l1_block(&self, blockid: &L1BlockId) -> WorkerResult<bitcoin::Block> {
        let l1_mf = self
            .l1man
            .get_block_manifest(blockid)
            .map_err(conv_db_err)?
            .ok_or(WorkerError::MissingL1Block(*blockid))?;

        self.handle
            .block_on(self.bitcoin_client.get_block_at(l1_mf.height()))
            .map_err(|_| WorkerError::MissingL1Block(*blockid))
    }

    fn get_latest_asm_state(&self) -> WorkerResult<Option<(L1BlockCommitment, AsmState)>> {
        self.asmman.fetch_most_recent_state().map_err(conv_db_err)
    }

    fn get_anchor_state(&self, blockid: &L1BlockCommitment) -> WorkerResult<AsmState> {
        self.asmman
            .get_state(*blockid)
            .map_err(conv_db_err)?
            .ok_or(WorkerError::MissingAsmState(*blockid.blkid()))
    }

    fn store_anchor_state(
        &self,
        blockid: &L1BlockCommitment,
        state: &AsmState,
    ) -> WorkerResult<()> {
        self.asmman
            .put_state(*blockid, state.clone())
            .map_err(conv_db_err)
    }

    fn get_network(&self) -> WorkerResult<bitcoin::Network> {
        self.handle
            .block_on(self.bitcoin_client.network())
            .map_err(|_| WorkerError::BtcClient)
    }
}

fn conv_db_err(_e: DbError) -> WorkerError {
    WorkerError::DbError
}
