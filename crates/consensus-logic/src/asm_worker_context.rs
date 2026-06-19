//! Context impl to instantiate ASM worker with.

use std::sync::Arc;

use bitcoin::block::Header;
use bitcoind_async_client::{client::Client, traits::Reader};
use strata_asm_common::{AsmManifest, AsmManifestHash, AuxData};
use strata_asm_worker::{
    AnchorStateStore, AsmState as WorkerAsmState, AuxDataStore, L1DataProvider, ManifestMmrStore,
    WorkerError, WorkerResult,
};
use strata_btc_types::L1BlockIdBitcoinExt;
use strata_common::retry::{policies::ExponentialBackoff, retry_with_backoff};
use strata_db_types::DbError;
use strata_identifiers::Hash;
use strata_primitives::prelude::*;
use strata_state::asm_state::AsmState as StorageAsmState;
use strata_storage::{AsmStateManager, L1BlockManager, MmrIndexHandle};
use tokio::runtime::Handle;
use tracing::{self, error};

#[expect(
    missing_debug_implementations,
    reason = "Inner types don't have Debug implementation"
)]
pub struct AsmWorkerCtx {
    handle: Handle,
    bitcoin_client: Arc<Client>,
    l1man: Arc<L1BlockManager>,
    asmman: Arc<AsmStateManager>,
    /// MMR handle for ASM manifest MMR
    mmr_handle: MmrIndexHandle,
}

impl AsmWorkerCtx {
    pub fn new(
        handle: Handle,
        bitcoin_client: Arc<Client>,
        l1man: Arc<L1BlockManager>,
        asmman: Arc<AsmStateManager>,
        mmr_handle: MmrIndexHandle,
    ) -> Self {
        Self {
            handle,
            bitcoin_client,
            l1man,
            asmman,
            mmr_handle,
        }
    }
}

impl L1DataProvider for AsmWorkerCtx {
    fn get_l1_block(&self, blockid: &L1BlockId) -> WorkerResult<bitcoin::Block> {
        let hash = blockid.to_block_hash();
        let backoff = ExponentialBackoff::new(200, 15, 10);
        retry_with_backoff("asm_get_l1_block", 10, &backoff, || {
            self.handle
                .block_on(self.bitcoin_client.get_block(&hash))
                .map_err(|e| {
                    tracing::warn!(%blockid, ?e, "failed to fetch L1 block for ASM");
                    WorkerError::MissingL1Block(*blockid)
                })
        })
    }

    fn get_l1_block_header(&self, blockid: &L1BlockId) -> WorkerResult<Header> {
        let hash = blockid.to_block_hash();
        let backoff = ExponentialBackoff::new(200, 15, 10);
        retry_with_backoff("asm_get_l1_block_header", 10, &backoff, || {
            self.handle
                .block_on(self.bitcoin_client.get_block_header(&hash))
                .map_err(|e| {
                    tracing::warn!(%blockid, ?e, "failed to fetch L1 block header for ASM");
                    WorkerError::MissingL1Block(*blockid)
                })
        })
    }

    // TODO(STR-3813): maintain a local L1 block-id -> height index instead of
    // round-tripping to the Bitcoin client for every lookup on the hot path.
    fn get_l1_block_height(&self, blockid: &L1BlockId) -> WorkerResult<u64> {
        let hash = blockid.to_block_hash();
        let backoff = ExponentialBackoff::new(200, 15, 10);
        retry_with_backoff("asm_get_l1_block_height", 10, &backoff, || {
            self.handle
                .block_on(self.bitcoin_client.get_block_height(&hash))
                .map_err(|e| {
                    tracing::warn!(%blockid, ?e, "failed to fetch L1 block height for ASM");
                    WorkerError::MissingL1Block(*blockid)
                })
        })
    }

    fn get_network(&self) -> WorkerResult<bitcoin::Network> {
        self.handle
            .block_on(self.bitcoin_client.network())
            .map_err(|e| WorkerError::BtcRpc(format!("network: {e}")))
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
                WorkerError::BitcoinTxNotFound(*txid)
            })?;

        let tx = raw_tx_response.0;

        Ok(RawBitcoinTx::from(tx))
    }
}

impl AnchorStateStore for AsmWorkerCtx {
    fn get_latest_asm_state(&self) -> WorkerResult<Option<(L1BlockCommitment, WorkerAsmState)>> {
        self.asmman
            .fetch_most_recent_state_blocking()
            .map_err(conv_db_err)
            .map(|state| state.map(|(block, state)| (block, storage_to_worker_state(state))))
    }

    fn get_anchor_state(&self, blockid: &L1BlockCommitment) -> WorkerResult<WorkerAsmState> {
        self.asmman
            .get_state_blocking(*blockid)
            .map_err(conv_db_err)?
            .map(storage_to_worker_state)
            .ok_or(WorkerError::MissingAsmState(*blockid.blkid()))
    }

    fn store_anchor_state(
        &self,
        blockid: &L1BlockCommitment,
        state: &WorkerAsmState,
    ) -> WorkerResult<()> {
        self.asmman
            .put_state_blocking(*blockid, worker_to_storage_state(state))
            .map_err(conv_db_err)
    }
}

impl ManifestMmrStore for AsmWorkerCtx {
    fn put_manifest(&self, manifest: AsmManifest) -> WorkerResult<()> {
        self.l1man.put_block_data(manifest).map_err(conv_db_err)
    }

    /// Writes a manifest hash as the height-indexed MMR leaf for `height`.
    ///
    /// The backing [`MmrIndexHandle`] is append-only (plus `pop`), so the
    /// height-indexed contract is mapped onto it positionally: with the genesis
    /// prefill in place, leaf index equals L1 height. A `height` at the current
    /// end appends; a `height` below it (an L1 reorg replacing the block at an
    /// already-seen height) pops every leaf from `height` up before re-appending,
    /// which is safe because the worker rewrites leaves from the reorg fork point
    /// forward and the dropped leaves belong to the abandoned branch; a `height`
    /// past the end is rejected to avoid a gap in the height-to-index mapping.
    ///
    /// # Crash safety
    ///
    /// The pop-then-append overwrite is not atomic, but it is crash-safe by the
    /// worker's commit ordering: `apply_block` records the manifest (this call)
    /// before it stores the anchor state, and a block counts as processed only
    /// once its anchor state is stored. A crash mid-overwrite leaves the block
    /// uncommitted, so the next sync re-runs it and re-invokes this with the same
    /// `(height, hash)`; popping back to `height` then appending reproduces the
    /// same leaf from any intermediate leaf count, so the re-run is idempotent.
    ///
    /// NOTE: a native height-indexed `put_leaf(height, hash)` on
    /// [`MmrIndexHandle`] (as the ASM runner's `SledAsmManifestMmrDb` provides)
    /// would collapse this to a single write.
    fn put_manifest_hash(&self, height: u64, hash: AsmManifestHash) -> WorkerResult<()> {
        let leaf = Hash::from(hash);
        let leaf_count = self.mmr_handle.get_num_leaves_blocking().map_err(|e| {
            error!(?e, "Failed to read manifest MMR leaf count");
            WorkerError::DbError
        })?;

        if height > leaf_count {
            return Err(WorkerError::ManifestIndexOutOfBound {
                index: height,
                max: leaf_count,
            });
        }

        for _ in height..leaf_count {
            self.mmr_handle.pop_leaf_blocking().map_err(|e| {
                error!(?e, "Failed to pop leaf from MMR");
                WorkerError::DbError
            })?;
        }

        self.mmr_handle.append_leaf_blocking(leaf).map_err(|e| {
            error!(?e, "Failed to append leaf to MMR");
            WorkerError::DbError
        })?;

        Ok(())
    }

    fn manifest_mmr_leaf_count(&self) -> WorkerResult<u64> {
        self.mmr_handle.get_num_leaves_blocking().map_err(|e| {
            error!(?e, "Failed to read manifest MMR leaf count");
            WorkerError::DbError
        })
    }

    fn generate_mmr_proof_at(
        &self,
        index: u64,
        at_leaf_count: u64,
    ) -> WorkerResult<strata_merkle::MerkleProofB32> {
        self.mmr_handle
            .generate_proof_at(index, at_leaf_count)
            .map_err(|e| {
                error!(?e, index, "Failed to generate MMR proof");
                WorkerError::MmrProofFailed { index }
            })
    }

    fn get_manifest_hash(&self, index: u64) -> WorkerResult<AsmManifestHash> {
        self.mmr_handle
            .get_leaf_blocking(index)
            .map_err(|e| {
                error!(?e, index, "Failed to get leaf hash from MMR");
                WorkerError::DbError
            })?
            .map(AsmManifestHash::from)
            .ok_or(WorkerError::ManifestHashNotFound { index })
    }
}

impl AuxDataStore for AsmWorkerCtx {
    fn store_aux_data(&self, blockid: &L1BlockCommitment, data: &AuxData) -> WorkerResult<()> {
        self.asmman
            .put_aux_data_blocking(*blockid, data.clone())
            .map_err(conv_db_err)
    }

    fn get_aux_data(&self, blockid: &L1BlockCommitment) -> WorkerResult<AuxData> {
        self.asmman
            .get_aux_data_blocking(*blockid)
            .map_err(conv_db_err)?
            .ok_or(WorkerError::MissingAuxData(*blockid))
    }
}

fn conv_db_err(_e: DbError) -> WorkerError {
    WorkerError::DbError
}

fn storage_to_worker_state(state: StorageAsmState) -> WorkerAsmState {
    WorkerAsmState::new(state.state().clone(), state.logs().clone())
}

fn worker_to_storage_state(state: &WorkerAsmState) -> StorageAsmState {
    StorageAsmState::new(state.state().clone(), state.logs().clone())
}
