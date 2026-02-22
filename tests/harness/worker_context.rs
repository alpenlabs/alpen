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
use strata_primitives::{
    buf::Buf32,
    hash::Hash,
    l1::{BitcoinTxid, L1BlockCommitment, L1BlockId},
};
use strata_state::asm_state::AsmState;
use tokio::{
    runtime::{Handle, Runtime},
    task::block_in_place,
};

/// Test implementation of WorkerContext for integration tests
///
/// Integrates with local regtest node via RPC client.
#[derive(Clone, Debug)]
pub struct TestAsmWorkerContext {
    /// Bitcoin RPC client for fetching blocks
    pub client: Arc<Client>,
    /// Block cache (optional - fetches from client if not cached)
    pub block_cache: Arc<Mutex<HashMap<L1BlockId, Block>>>,
    /// ASM states indexed by L1 block commitment
    pub asm_states: Arc<Mutex<HashMap<L1BlockCommitment, AsmState>>>,
    /// Latest ASM state
    pub latest_asm_state: Arc<Mutex<Option<(L1BlockCommitment, AsmState)>>>,
    /// In-memory MMR for manifest hashes (leaf index -> hash)
    pub mmr_leaves: Arc<Mutex<Vec<[u8; 32]>>>,
    /// Manifest hash lookup by index
    pub manifest_hashes: Arc<Mutex<HashMap<u64, [u8; 32]>>>,
    /// Stored manifests in insertion order
    pub manifests: Arc<Mutex<Vec<AsmManifest>>>,
}

impl TestAsmWorkerContext {
    /// Create a new test context with a Bitcoin RPC client
    pub fn new(client: Client) -> Self {
        Self {
            client: Arc::new(client),
            block_cache: Arc::new(Mutex::new(HashMap::new())),
            asm_states: Arc::new(Mutex::new(HashMap::new())),
            latest_asm_state: Arc::new(Mutex::new(None)),
            mmr_leaves: Arc::new(Mutex::new(Vec::new())),
            manifest_hashes: Arc::new(Mutex::new(HashMap::new())),
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

        // If not cached, fetch from regtest (synchronously)
        let block_hash = blockid.to_block_hash();

        // Try to use current runtime if available, otherwise create a new one
        let block = match Handle::try_current() {
            Ok(handle) => {
                // We're in a Tokio context, use block_in_place
                block_in_place(|| {
                    handle.block_on(async { self.client.get_block(&block_hash).await })
                })
            }
            Err(_) => {
                // No runtime available, create a temporary one
                let rt = Runtime::new().map_err(|_| WorkerError::MissingL1Block(*blockid))?;
                rt.block_on(async { self.client.get_block(&block_hash).await })
            }
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
        // Convert BitcoinTxid to Txid
        let txid_inner: Txid = (*txid).into();

        // Fetch transaction from regtest synchronously
        // Try to use current runtime if available, otherwise create a new one
        let raw_tx_result = match Handle::try_current() {
            Ok(handle) => {
                // We're in a Tokio context, use block_in_place
                block_in_place(|| {
                    handle.block_on(async {
                        self.client
                            .get_raw_transaction_verbosity_zero(&txid_inner)
                            .await
                    })
                })
            }
            Err(_) => {
                // No runtime available, create a temporary one
                let rt = Runtime::new().map_err(|_| WorkerError::BitcoinTxNotFound(*txid))?;
                rt.block_on(async {
                    self.client
                        .get_raw_transaction_verbosity_zero(&txid_inner)
                        .await
                })
            }
        }
        .map_err(|_| WorkerError::BitcoinTxNotFound(*txid))?;

        // Extract the transaction and convert to RawBitcoinTx
        let tx = raw_tx_result.0;

        Ok(RawBitcoinTx::from(tx))
    }

    fn append_manifest_to_mmr(&self, manifest_hash: Hash) -> WorkerResult<u64> {
        let mut leaves = self.mmr_leaves.lock().unwrap();
        let index = leaves.len() as u64;
        let hash_bytes = *manifest_hash.as_ref();
        leaves.push(hash_bytes);
        // Keep manifest_hashes in sync so get_manifest_hash lookups work.
        self.manifest_hashes
            .lock()
            .unwrap()
            .insert(index, hash_bytes);
        Ok(index)
    }

    fn generate_mmr_proof(&self, _index: u64) -> WorkerResult<strata_merkle::MerkleProofB32> {
        // TODO: Implement proper MMR proof generation when needed
        // For now, this is not used in basic tests
        Err(WorkerError::Unimplemented)
    }

    fn get_manifest_hash(&self, index: u64) -> WorkerResult<Option<Hash>> {
        Ok(self
            .manifest_hashes
            .lock()
            .unwrap()
            .get(&index)
            .map(|h| Buf32::from(*h)))
    }

    fn store_l1_manifest(&self, manifest: AsmManifest) -> WorkerResult<()> {
        self.manifests.lock().unwrap().push(manifest);
        Ok(())
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
