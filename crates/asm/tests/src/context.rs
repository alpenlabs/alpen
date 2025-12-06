use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use bitcoin::{Block, Network};
use strata_asm_worker::{WorkerContext, WorkerError, WorkerResult};
use strata_primitives::l1::{L1BlockCommitment, L1BlockId};
use strata_state::asm_state::AsmState;

#[derive(Clone, Default)]
pub struct MockWorkerContext {
    pub blocks: Arc<Mutex<HashMap<L1BlockId, Block>>>,
    pub asm_states: Arc<Mutex<HashMap<L1BlockCommitment, AsmState>>>,
    pub latest_asm_state: Arc<Mutex<Option<(L1BlockCommitment, AsmState)>>>,
}

impl MockWorkerContext {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl WorkerContext for MockWorkerContext {
    fn get_l1_block(&self, blockid: &L1BlockId) -> WorkerResult<Block> {
        self.blocks
            .lock()
            .unwrap()
            .get(blockid)
            .cloned()
            .ok_or(WorkerError::MissingL1Block(*blockid))
    }

    fn get_anchor_state(&self, blockid: &L1BlockCommitment) -> WorkerResult<AsmState> {
        self.asm_states
            .lock()
            .unwrap()
            .get(blockid)
            .cloned()
            .ok_or(WorkerError::MissingAsmState(*blockid.blkid()))
    }

    fn get_latest_asm_state(&self) -> WorkerResult<Option<(L1BlockCommitment, AsmState)>> {
        Ok(self.latest_asm_state.lock().unwrap().clone())
    }

    fn store_anchor_state(
        &self,
        blockid: &L1BlockCommitment,
        state: &AsmState,
    ) -> WorkerResult<()> {
        self.asm_states
            .lock()
            .unwrap()
            .insert(blockid.clone(), state.clone());
        *self.latest_asm_state.lock().unwrap() = Some((*blockid, state.clone()));
        Ok(())
    }

    fn get_network(&self) -> WorkerResult<Network> {
        Ok(Network::Regtest)
    }
}
