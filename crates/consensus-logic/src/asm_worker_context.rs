//! Context impl to instantiate ASM worker with.

use std::sync::Arc;

use bitcoind_async_client::{client::Client, traits::Reader};
use strata_asm_worker::{WorkerContext, WorkerError, WorkerResult};
use strata_db_types::DbError;
use strata_primitives::prelude::*;
use strata_state::asm_state::AsmState;
use strata_storage::{AsmStateManager, L1BlockManager, MmrManager};
use tokio::runtime::Handle;
use tracing;

#[expect(
    missing_debug_implementations,
    reason = "Inner types don't have Debug implementation"
)]
pub struct AsmWorkerCtx {
    handle: Handle,
    bitcoin_client: Arc<Client>,
    l1man: Arc<L1BlockManager>,
    asmman: Arc<AsmStateManager>,
    /// MMR manager for proof generation
    mmr_manager: Arc<MmrManager>,
}

impl AsmWorkerCtx {
    pub fn new(
        handle: Handle,
        bitcoin_client: Arc<Client>,
        l1man: Arc<L1BlockManager>,
        asmman: Arc<AsmStateManager>,
        mmr_manager: Arc<MmrManager>,
    ) -> Self {
        Self {
            handle,
            bitcoin_client,
            l1man,
            asmman,
            mmr_manager,
        }
    }
}

impl WorkerContext for AsmWorkerCtx {
    fn get_l1_block(&self, blockid: &L1BlockId) -> WorkerResult<bitcoin::Block> {
        // With ASM manifests, we don't store height in the manifest anymore
        // We need to search the canonical chain to find the height
        let tip_opt = self.l1man.get_canonical_chain_tip().map_err(conv_db_err)?;
        let Some((tip_height, _)) = tip_opt else {
            return Err(WorkerError::MissingL1Block(*blockid));
        };

        // Search backwards from tip to find the block
        for height in (0..=tip_height).rev() {
            if let Some(bid) = self
                .l1man
                .get_canonical_blockid_at_height(height)
                .map_err(conv_db_err)?
            {
                if bid == *blockid {
                    return self
                        .handle
                        .block_on(self.bitcoin_client.get_block_at(height))
                        .map_err(|_| WorkerError::MissingL1Block(*blockid));
                }
            }
        }

        Err(WorkerError::MissingL1Block(*blockid))
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

    fn store_l1_manifest(&self, manifest: strata_asm_common::AsmManifest) -> WorkerResult<()> {
        self.l1man.put_block_data(manifest).map_err(conv_db_err)
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

        let tx = raw_tx_response.0;
        Ok(RawBitcoinTx::from(tx))
    }

    fn append_manifest_to_mmr(&self, manifest_hash: [u8; 32]) -> WorkerResult<u64> {
        self.mmr_manager
            .append_leaf_blocking(manifest_hash)
            .map_err(|e| {
                tracing::error!(?e, "Failed to append leaf to MMR");
                WorkerError::DbError
            })
    }

    fn store_manifest_hash(&self, index: u64, hash: [u8; 32]) -> WorkerResult<()> {
        self.asmman
            .store_manifest_hash(index, Buf32(hash))
            .map_err(conv_db_err)
    }

    fn generate_mmr_proof(&self, index: u64) -> WorkerResult<strata_merkle::MerkleProofB32> {
        self.mmr_manager.generate_proof(index).map_err(|e| {
            tracing::error!(?e, index, "Failed to generate MMR proof");
            WorkerError::MmrProofFailed { index }
        })
    }

    fn get_manifest_hash(&self, index: u64) -> WorkerResult<Option<[u8; 32]>> {
        self.asmman
            .get_manifest_hash(index)
            .map(|opt| opt.map(|buf| buf.0))
            .map_err(conv_db_err)
    }
}

fn conv_db_err(_e: DbError) -> WorkerError {
    WorkerError::DbError
}
