//! ASM worker context implementation for integration tests.
//!
//! Provides `TestAsmWorkerContext` which implements the `WorkerContext` trait,
//! allowing the ASM worker to fetch blocks and store state during tests.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use bitcoin::{block::Header, Block, BlockHash, Network, Txid};
use bitcoind_async_client::{traits::Reader, Client};
use strata_asm_manifest_types::AsmManifest;
use strata_asm_worker::{WorkerContext, WorkerError, WorkerResult};
use strata_btc_types::{BlockHashExt, GenesisL1View, L1BlockIdBitcoinExt, RawBitcoinTx};
use strata_merkle::{CompactMmr64, MerkleProofB32, Mmr, Sha256Hasher};
use strata_primitives::{
    buf::Buf32,
    hash::Hash,
    l1::{BitcoinTxid, L1BlockCommitment, L1BlockId},
};
use strata_state::asm_state::AsmState;
use tokio::{runtime::Handle, task::block_in_place};

/// Test implementation of WorkerContext for integration tests
///
/// Integrates with local regtest node via RPC client.
#[derive(Clone, Debug)]
pub struct TestAsmWorkerContext {
    /// Bitcoin RPC client for fetching blocks
    pub client: Arc<Client>,
    /// Tokio runtime handle from the test runtime, used for async operations
    /// from the worker's dedicated OS thread (which has no tokio context).
    pub tokio_handle: Handle,
    /// Block cache (optional - fetches from client if not cached)
    pub block_cache: Arc<Mutex<HashMap<L1BlockId, Block>>>,
    /// ASM states indexed by L1 block commitment
    pub asm_states: Arc<Mutex<HashMap<L1BlockCommitment, AsmState>>>,
    /// Latest ASM state
    pub latest_asm_state: Arc<Mutex<Option<(L1BlockCommitment, AsmState)>>>,
    /// In-memory MMR leaves in insertion order.
    pub mmr_leaves: Arc<Mutex<Vec<[u8; 32]>>>,
    /// Stored manifests in insertion order
    pub manifests: Arc<Mutex<Vec<AsmManifest>>>,
}

impl TestAsmWorkerContext {
    /// Create a new test context with a Bitcoin RPC client.
    ///
    /// Captures the current tokio runtime handle so the worker's dedicated OS
    /// thread can drive async operations on the original runtime (where the
    /// HTTP client's connection pool lives).
    pub fn new(client: Client) -> Self {
        Self {
            client: Arc::new(client),
            tokio_handle: Handle::current(),
            block_cache: Arc::new(Mutex::new(HashMap::new())),
            asm_states: Arc::new(Mutex::new(HashMap::new())),
            latest_asm_state: Arc::new(Mutex::new(None)),
            mmr_leaves: Arc::new(Mutex::new(Vec::new())),
            manifests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Fetch a block from regtest by hash, caching it for future use
    pub async fn fetch_and_cache_block(&self, block_hash: BlockHash) -> anyhow::Result<Block> {
        let block = self.client.get_block(&block_hash).await?;
        let block_id = block_hash.to_l1_block_id();
        self.block_cache
            .lock()
            .unwrap()
            .insert(block_id, block.clone());
        Ok(block)
    }
}

impl WorkerContext for TestAsmWorkerContext {
    fn get_l1_block(&self, blockid: &L1BlockId) -> WorkerResult<Block> {
        // Try cache first
        if let Some(block) = self.block_cache.lock().unwrap().get(blockid).cloned() {
            return Ok(block);
        }

        // Fetch from regtest. We must handle two calling contexts:
        // 1. From within a tokio runtime (test thread) — use `block_in_place` to avoid "cannot
        //    start a runtime from within a runtime" panic.
        // 2. From the worker's dedicated OS thread (spawned by `spawn_critical`, no tokio context)
        //    — use the stored handle to drive the future on the original runtime where the HTTP
        //    client's connection pool lives.
        let block_hash = blockid.to_block_hash();
        let client = self.client.clone();
        let fetch = || async { client.get_block(&block_hash).await };
        let block = if Handle::try_current().is_ok() {
            block_in_place(|| self.tokio_handle.block_on(fetch()))
        } else {
            self.tokio_handle.block_on(fetch())
        }
        .map_err(|_| WorkerError::MissingL1Block(*blockid))?;

        // Cache for future use
        self.block_cache
            .lock()
            .unwrap()
            .insert(*blockid, block.clone());

        Ok(block)
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
            .insert(*blockid, state.clone());
        *self.latest_asm_state.lock().unwrap() = Some((*blockid, state.clone()));
        Ok(())
    }

    fn get_network(&self) -> WorkerResult<Network> {
        Ok(Network::Regtest)
    }

    fn get_bitcoin_tx(&self, txid: &BitcoinTxid) -> WorkerResult<RawBitcoinTx> {
        let txid_inner: Txid = (*txid).into();

        // See `get_l1_block` for the two-context branching rationale.
        let client = self.client.clone();
        let fetch = || async move { client.get_raw_transaction_verbosity_zero(&txid_inner).await };
        let raw_tx_result = if Handle::try_current().is_ok() {
            block_in_place(|| self.tokio_handle.block_on(fetch()))
        } else {
            self.tokio_handle.block_on(fetch())
        }
        .map_err(|_| WorkerError::BitcoinTxNotFound(*txid))?;

        Ok(RawBitcoinTx::from(raw_tx_result.0))
    }

    fn append_manifest_to_mmr(&self, manifest_hash: Hash) -> WorkerResult<u64> {
        let hash_bytes: [u8; 32] = *manifest_hash.as_ref();
        let mut leaves = self.mmr_leaves.lock().unwrap();
        let leaf_index = leaves.len() as u64;
        leaves.push(hash_bytes);
        Ok(leaf_index)
    }

    fn generate_mmr_proof_at(
        &self,
        index: u64,
        at_leaf_count: u64,
    ) -> WorkerResult<strata_merkle::MerkleProofB32> {
        let leaves = self.mmr_leaves.lock().unwrap();
        if index >= at_leaf_count || at_leaf_count > leaves.len() as u64 {
            return Err(WorkerError::MmrProofFailed { index });
        }

        let mut compact = CompactMmr64::new(64);
        let at_leaf_count = at_leaf_count as usize;
        let mut proof_list = Vec::with_capacity(at_leaf_count);
        for leaf in leaves.iter().take(at_leaf_count) {
            let proof = Mmr::<Sha256Hasher>::add_leaf_updating_proof_list(
                &mut compact,
                *leaf,
                &mut proof_list,
            )
            .map_err(|_| WorkerError::MmrProofFailed { index })?;
            proof_list.push(proof);
        }

        proof_list
            .get(index as usize)
            .map(MerkleProofB32::from_generic)
            .ok_or(WorkerError::MmrProofFailed { index })
    }

    fn get_manifest_hash(&self, index: u64) -> WorkerResult<Option<Hash>> {
        Ok(self
            .mmr_leaves
            .lock()
            .unwrap()
            .get(index as usize)
            .copied()
            .map(Buf32::from))
    }

    fn store_l1_manifest(&self, manifest: AsmManifest) -> WorkerResult<()> {
        self.manifests.lock().unwrap().push(manifest);
        Ok(())
    }

    fn store_aux_data(
        &self,
        _blockid: &L1BlockCommitment,
        _data: &strata_asm_common::AuxData,
    ) -> WorkerResult<()> {
        Ok(())
    }

    fn get_aux_data(
        &self,
        _blockid: &L1BlockCommitment,
    ) -> WorkerResult<Option<strata_asm_common::AuxData>> {
        Ok(None)
    }

    fn has_l1_manifest(&self, blockid: &L1BlockId) -> WorkerResult<bool> {
        Ok(self
            .manifests
            .lock()
            .unwrap()
            .iter()
            .any(|m| m.blkid() == blockid))
    }
}

/// Helper to construct GenesisL1View from a block hash using the client.
pub async fn get_genesis_l1_view(
    client: &Client,
    hash: &BlockHash,
) -> anyhow::Result<GenesisL1View> {
    let header: Header = client.get_block_header(hash).await?;
    let height = client.get_block_height(hash).await?;

    // Construct L1BlockCommitment
    let blkid = header.block_hash().to_l1_block_id();
    let blk_commitment = L1BlockCommitment::new(height as u32, blkid);

    // Create dummy/default values for other fields
    let next_target = header.bits.to_consensus();
    let epoch_start_timestamp = header.time;
    let last_11_timestamps = [header.time - 1; 11]; // simplified: ensure median < tip time

    Ok(GenesisL1View {
        blk: blk_commitment,
        next_target,
        epoch_start_timestamp,
        last_11_timestamps,
    })
}
