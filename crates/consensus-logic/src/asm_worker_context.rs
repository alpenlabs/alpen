//! Context impl to instantiate ASM worker with.

use std::sync::Arc;

use bitcoind_async_client::{client::Client, traits::Reader};
use strata_asm_worker::{WorkerContext, WorkerError, WorkerResult};
use strata_db_types::DbError;
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
    /// Lazily initialized MMR database (created on first use)
    mmr_db: std::sync::Mutex<Option<strata_storage::mmr_db::SledMmrDb>>,
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
            mmr_db: std::sync::Mutex::new(None),
        }
    }

    /// Gets or initializes the MMR database
    fn get_mmr_db(&self) -> WorkerResult<strata_storage::mmr_db::SledMmrDb> {
        let mut mmr_db_lock = self.mmr_db.lock().map_err(|_| WorkerError::DbError)?;

        if mmr_db_lock.is_none() {
            // Initialize MMR database on first use
            let db = self
                .asmman
                .create_mmr_database()
                .map_err(|_| WorkerError::DbError)?;
            *mmr_db_lock = Some(db);
        }

        // Clone the database (it's Arc-based internally via sled trees)
        mmr_db_lock.as_ref().cloned().ok_or(WorkerError::DbError)
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

    fn get_bitcoin_tx(&self, txid: &strata_btc_types::BitcoinTxid) -> WorkerResult<RawBitcoinTx> {
        let bitcoin_txid = txid.inner();

        let raw_tx_response = self
            .handle
            .block_on(
                self.bitcoin_client
                    .get_raw_transaction_verbosity_zero(&bitcoin_txid),
            )
            .map_err(|e| {
                tracing::warn!(?txid, ?e, "Failed to fetch Bitcoin transaction");
                WorkerError::BitcoinTxNotFound(txid.clone())
            })?;

        let tx = raw_tx_response.transaction().map_err(|e| {
            tracing::error!(?txid, ?e, "Failed to decode transaction");
            WorkerError::BitcoinTxNotFound(txid.clone())
        })?;

        Ok(RawBitcoinTx::from(tx))
    }

    fn append_manifest_to_mmr(&self, manifest_hash: [u8; 32]) -> WorkerResult<u64> {
        let mut mmr_db = self.get_mmr_db()?;
        use strata_storage::mmr_db::MmrDatabase;
        mmr_db
            .append_leaf(manifest_hash)
            .map_err(|_| WorkerError::DbError)
    }

    fn store_manifest_hash(&self, index: u64, hash: [u8; 32]) -> WorkerResult<()> {
        self.asmman
            .store_manifest_hash(index, hash)
            .map_err(conv_db_err)
    }

    fn get_mmr_database(&self) -> WorkerResult<strata_storage::mmr_db::SledMmrDb> {
        self.get_mmr_db()
    }

    fn get_manifest_hash(&self, index: u64) -> WorkerResult<Option<[u8; 32]>> {
        self.asmman.get_manifest_hash(index).map_err(conv_db_err)
    }
}

fn conv_db_err(_e: DbError) -> WorkerError {
    WorkerError::DbError
}
