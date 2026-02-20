//! Test harness for running ASM worker as a service with Bitcoin regtest
//!
//! This module provides core infrastructure for integration tests:
//! - Bitcoin regtest node management
//! - ASM worker service lifecycle
//! - Block mining and submission
//! - State query utilities
//! - Generic SPS-50 transaction building
//!
//! # Architecture
//!
//! The harness provides subprotocol-agnostic infrastructure. Subprotocol-specific
//! functionality is added via extension traits in separate modules:
//! - `AdminExt` in `admin.rs` - admin subprotocol operations
//! - `CheckpointExt` in `checkpoint.rs` - checkpoint subprotocol operations
//! - (future) `BridgeExt` in `bridge.rs` - bridge subprotocol operations
//!
//! # Example
//!
//! ```ignore
//! use harness::test_harness::create_test_harness;
//! use harness::admin::{AdminExt, sequencer_update};
//!
//! let harness = create_test_harness().await?;
//! let mut ctx = harness.admin_context();  // From AdminExt
//! harness.submit_admin_action(&mut ctx, sequencer_update([1u8; 32])).await?;
//! ```

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use bitcoin::{
    absolute::LockTime,
    blockdata::script,
    key::UntweakedKeypair,
    opcodes::{
        all::{OP_ENDIF, OP_IF},
        OP_FALSE,
    },
    script::PushBytesBuf,
    secp256k1::{All, Secp256k1, XOnlyPublicKey},
    taproot::{LeafVersion, TaprootBuilder, TaprootSpendInfo},
    transaction::Version,
    Address, Amount, Block, BlockHash, Network, OutPoint, ScriptBuf, Sequence, Transaction, TxIn,
    TxOut, Txid, Witness,
};
use bitcoind_async_client::{
    traits::{Reader, Wallet},
    Client,
};
use corepc_node::Node;
use rand::RngCore;
use strata_asm_worker::{AsmWorkerBuilder, AsmWorkerHandle, WorkerContext};
use strata_l1_txfmt::{ParseConfig, SubprotocolId, TagDataRef, TxType};
use strata_params::RollupParams;
use strata_primitives::{
    buf::Buf32,
    l1::{L1BlockCommitment, L1BlockId},
};
use strata_state::{asm_state::AsmState, BlockSubmitter};
use strata_tasks::{TaskExecutor, TaskManager};
use tokio::{runtime::Handle, task::block_in_place, time::sleep};

use super::worker_context::{get_genesis_l1_view, TestAsmWorkerContext};

// ============================================================================
// Test Harness
// ============================================================================

/// Test harness that manages ASM worker service and Bitcoin regtest.
///
/// This struct provides core infrastructure for integration tests:
/// - Bitcoin regtest node and RPC client
/// - ASM worker service lifecycle
/// - Block mining and submission
/// - State queries
/// - Generic SPS-50 transaction building
///
/// Subprotocol-specific methods are provided via extension traits
/// (`AdminExt`, `CheckpointExt`, etc.) implemented in their respective modules.
#[derive(Debug)]
pub struct AsmTestHarness {
    /// Bitcoin regtest node
    pub bitcoind: Node,
    /// Bitcoin RPC client
    pub client: Arc<Client>,
    /// ASM worker handle for submitting blocks
    pub asm_handle: AsmWorkerHandle,
    /// ASM worker context for querying state
    pub context: TestAsmWorkerContext,
    /// Rollup parameters
    pub params: Arc<RollupParams>,
    /// Task executor for spawning tasks
    pub executor: TaskExecutor,
    /// Genesis block height
    pub genesis_height: u64,
}

impl AsmTestHarness {
    /// Default transaction fee.
    pub const DEFAULT_FEE: Amount = Amount::from_sat(1000);

    /// Create a new test harness with ASM worker service.
    ///
    /// This will:
    /// 1. Start Bitcoin regtest node
    /// 2. Mine initial blocks to genesis height
    /// 3. Initialize ASM worker context
    /// 4. Launch ASM worker service in background
    ///
    /// # Arguments
    /// * `genesis_height` - Height of the genesis block (e.g., 101)
    pub async fn new(genesis_height: u64) -> anyhow::Result<Self> {
        // 1. Start Bitcoin regtest
        let (bitcoind, client) = strata_test_utils_btcio::get_bitcoind_and_client();
        let client = Arc::new(client);

        // 2. Mine blocks to genesis height
        strata_test_utils_btcio::mine_blocks(&bitcoind, &client, genesis_height as usize, None)
            .await?;

        let genesis_hash = client.get_block_hash(genesis_height).await?;

        // 3. Setup parameters
        let mut params = strata_test_utils_l2::gen_params().rollup;
        params.network = Network::Regtest;
        let genesis_view = get_genesis_l1_view(&client, &genesis_hash).await?;
        params.genesis_l1_view = genesis_view;
        let params = Arc::new(params);

        // 4. Create worker context
        let context = TestAsmWorkerContext::new((*client).clone());

        // 5. Create task executor
        let task_manager = TaskManager::new(Handle::current());
        let executor = task_manager.create_executor();

        // 6. Launch ASM worker service
        let asm_handle = AsmWorkerBuilder::new()
            .with_context(context.clone())
            .with_params(params.clone())
            .launch(&executor)?;

        let harness = Self {
            bitcoind,
            client,
            asm_handle,
            context,
            params,
            executor,
            genesis_height,
        };

        // Submit genesis block to ASM worker
        let genesis_block_id = L1BlockId::from(genesis_hash);
        let genesis_commitment = L1BlockCommitment::new(genesis_height as u32, genesis_block_id);

        // Fetch and cache genesis block
        let _genesis_block = harness.context.fetch_and_cache_block(genesis_hash).await?;

        // Submit genesis block
        block_in_place(|| harness.asm_handle.submit_block(genesis_commitment))?;

        // Wait for ASM worker to process genesis block
        // Poll until we have an initial state (with timeout)
        let start = Instant::now();
        let timeout = Duration::from_secs(10);
        loop {
            if harness.get_latest_asm_state().ok().flatten().is_some() {
                break;
            }
            if start.elapsed() > timeout {
                anyhow::bail!("Timeout waiting for ASM worker to process genesis block");
            }
            sleep(Duration::from_millis(100)).await;
        }

        Ok(harness)
    }

    // ========================================================================
    // Block Mining
    // ========================================================================

    /// Mine a block and wait for ASM worker to process it.
    ///
    /// This will:
    /// 1. Mine a block on regtest (coinbase to given address or a new one)
    /// 2. Fetch and cache the block
    /// 3. Submit the block commitment to ASM worker
    /// 4. Wait until ASM worker has processed the block
    ///
    /// When this method returns, the block is guaranteed to be processed by ASM.
    ///
    /// # Returns
    /// The block hash of the mined block
    pub async fn mine_block(&self, address: Option<bitcoin::Address>) -> anyhow::Result<BlockHash> {
        // Mine block
        let address = match address {
            Some(addr) => addr,
            None => self.client.get_new_address().await?,
        };

        let block_hashes =
            strata_test_utils_btcio::mine_blocks(&self.bitcoind, &self.client, 1, Some(address))
                .await?;

        let block_hash = block_hashes[0];

        // Fetch and cache the block
        let _block = self.context.fetch_and_cache_block(block_hash).await?;

        // Get block height
        let height = self.client.get_block_height(&block_hash).await?;

        // Create L1BlockCommitment and submit to ASM worker
        let block_id = block_hash.into();
        let block_commitment = L1BlockCommitment::new(height as u32, block_id);

        // Use block_in_place to submit synchronously within async context
        block_in_place(|| self.asm_handle.submit_block(block_commitment))?;

        // Wait for ASM worker to process the block
        self.wait_for_height(height, Duration::from_secs(5)).await?;

        Ok(block_hash)
    }

    /// Mine multiple blocks, waiting for each to be processed by ASM.
    ///
    /// # Arguments
    /// * `count` - Number of blocks to mine
    ///
    /// # Returns
    /// Vector of block hashes
    pub async fn mine_blocks(&self, count: usize) -> anyhow::Result<Vec<BlockHash>> {
        let mut hashes = Vec::new();
        for _ in 0..count {
            let hash = self.mine_block(None).await?;
            hashes.push(hash);
        }
        Ok(hashes)
    }

    // ========================================================================
    // Transaction Submission
    // ========================================================================

    /// Submit a transaction to Bitcoin regtest mempool.
    ///
    /// Note: The transaction must be valid and properly funded
    pub async fn submit_transaction(
        &self,
        tx: &bitcoin::Transaction,
    ) -> anyhow::Result<bitcoin::Txid> {
        let result = self.bitcoind.client.send_raw_transaction(tx)?;
        Ok(result.0.parse()?)
    }

    /// Submit a transaction to mempool and mine blocks until it's included.
    ///
    /// Keeps mining blocks until the transaction is confirmed, then waits for
    /// ASM worker to process the block before returning.
    ///
    /// # Returns
    /// The block hash containing the transaction
    pub async fn submit_and_mine_tx(&self, tx: &Transaction) -> anyhow::Result<BlockHash> {
        let txid = self.submit_transaction(tx).await?;

        // Mine blocks until tx is confirmed
        for _ in 0..10 {
            let block_hash = self.mine_block(None).await?;
            let block = self.context.fetch_and_cache_block(block_hash).await?;

            // Check if our tx is in this block
            if block.txdata.iter().any(|t| t.compute_txid() == txid) {
                return Ok(block_hash);
            }
        }

        anyhow::bail!("Transaction {txid} not included after 10 blocks")
    }

    // ========================================================================
    // Waiting & Synchronization
    // ========================================================================

    /// Wait for ASM state to advance beyond a given height.
    ///
    /// Polls the ASM state until it processes a block at or above the target height,
    /// or times out after the specified duration.
    pub async fn wait_for_height(
        &self,
        target_height: u64,
        timeout: Duration,
    ) -> anyhow::Result<()> {
        let start = Instant::now();
        loop {
            if start.elapsed() > timeout {
                anyhow::bail!("Timeout waiting for height {target_height}");
            }

            if let Some((commitment, _state)) = self.context.get_latest_asm_state()? {
                let current_height = commitment.height() as u64;
                if current_height >= target_height {
                    return Ok(());
                }
            }

            sleep(Duration::from_millis(50)).await;
        }
    }

    /// Wait for a specific block to be processed by ASM worker.
    ///
    /// Polls until the ASM state for the given block exists, or times out.
    pub async fn wait_for_block(
        &self,
        blockid: &L1BlockCommitment,
        timeout: Duration,
    ) -> anyhow::Result<AsmState> {
        let start = Instant::now();
        loop {
            if start.elapsed() > timeout {
                anyhow::bail!("Timeout waiting for block {:?}", blockid);
            }

            match self.context.get_anchor_state(blockid) {
                Ok(state) => return Ok(state),
                Err(_) => {
                    sleep(Duration::from_millis(50)).await;
                }
            }
        }
    }

    /// Wait for the next block to be processed.
    pub async fn wait_for_next_block(&self) -> anyhow::Result<()> {
        let current = self.get_processed_height()?;
        self.wait_for_height(current + 1, Duration::from_secs(5))
            .await
    }

    // ========================================================================
    // State Queries
    // ========================================================================

    /// Get the current chain tip height from Bitcoin.
    pub async fn get_chain_tip(&self) -> anyhow::Result<u64> {
        Ok(self.client.get_blockchain_info().await?.blocks.into())
    }

    /// Get the current processed height from ASM state.
    pub fn get_processed_height(&self) -> anyhow::Result<u64> {
        let (commitment, _) = self
            .get_latest_asm_state()?
            .ok_or_else(|| anyhow::anyhow!("No ASM state available"))?;
        Ok(commitment.height() as u64)
    }

    /// Get the latest ASM state from the worker context.
    pub fn get_latest_asm_state(&self) -> anyhow::Result<Option<(L1BlockCommitment, AsmState)>> {
        Ok(self.context.get_latest_asm_state()?)
    }

    /// Get ASM state at a specific block.
    pub fn get_asm_state_at(&self, blockid: &L1BlockCommitment) -> anyhow::Result<AsmState> {
        Ok(self.context.get_anchor_state(blockid)?)
    }

    /// Get a block from the cache or Bitcoin.
    pub async fn get_block(&self, block_hash: BlockHash) -> anyhow::Result<Block> {
        self.context.fetch_and_cache_block(block_hash).await
    }

    /// Get the number of MMR leaves (manifest hashes) stored.
    pub fn get_mmr_leaf_count(&self) -> usize {
        self.context.mmr_leaves.lock().unwrap().len()
    }

    /// Get a manifest hash by index.
    pub fn get_manifest_hash(&self, index: u64) -> anyhow::Result<Option<Buf32>> {
        Ok(self.context.get_manifest_hash(index)?)
    }

    /// Get a snapshot of all stored manifests.
    pub fn get_stored_manifests(&self) -> Vec<strata_asm_manifest_types::AsmManifest> {
        self.context.manifests.lock().unwrap().clone()
    }

    /// Get a snapshot of all external MMR leaf hashes.
    pub fn get_mmr_leaves(&self) -> Vec<[u8; 32]> {
        self.context.mmr_leaves.lock().unwrap().clone()
    }

    // ========================================================================
    // Funding & Wallet
    // ========================================================================

    /// Create a funding UTXO for transaction building.
    ///
    /// This uses Bitcoin Core's send_to_address to create a new UTXO
    /// with the specified amount, which can then be used as an input.
    ///
    /// # Arguments
    /// * `address` - Address to send funds to
    /// * `amount` - Amount to send (including fees)
    ///
    /// # Returns
    /// (txid, vout) of the created UTXO
    async fn create_funding_utxo(
        &self,
        address: &Address,
        amount: Amount,
    ) -> anyhow::Result<(Txid, u32)> {
        let funding_txid_str = self
            .bitcoind
            .client
            .send_to_address(address, amount)?
            .0
            .to_string();
        let funding_txid: Txid = funding_txid_str.parse()?;

        let funding_tx = self
            .client
            .get_raw_transaction_verbosity_zero(&funding_txid)
            .await?
            .0;

        let prev_vout = funding_tx
            .output
            .iter()
            .enumerate()
            .find(|(_, output)| output.script_pubkey == address.script_pubkey())
            .map(|(idx, _)| idx as u32)
            .ok_or_else(|| anyhow::anyhow!("Could not find output in funding transaction"))?;

        Ok((funding_txid, prev_vout))
    }

    // ========================================================================
    // SPS-50 Transaction Building
    // ========================================================================

    /// Build a funded SPS-50 envelope transaction with real UTXOs.
    ///
    /// This creates a proper Bitcoin transaction that:
    /// 1. Spends a real UTXO (funded by mining blocks)
    /// 2. Contains the payload in taproot envelope script format
    /// 3. Has SPS-50 compliant OP_RETURN tag
    ///
    /// # Arguments
    /// * `subprotocol_id` - SPS-50 subprotocol ID (0=admin, 1=checkpoint, 2=bridge)
    /// * `tx_type` - Transaction type within the subprotocol
    /// * `payload` - Serialized payload to embed in witness
    pub async fn build_envelope_tx(
        &self,
        subprotocol_id: SubprotocolId,
        tx_type: TxType,
        payload: Vec<u8>,
    ) -> anyhow::Result<Transaction> {
        let fee = Self::DEFAULT_FEE;
        // Calculate funding amount needed (outputs + fee buffer)
        let dust_amount = Amount::from_sat(1000);
        let funding_amount = fee + dust_amount + Amount::from_sat(1000);

        // Generate taproot keypair and build reveal script
        let secp = Secp256k1::new();
        let mut rng = rand::thread_rng();
        let mut key_bytes = [0u8; 32];
        rng.fill_bytes(&mut key_bytes);
        let keypair = UntweakedKeypair::from_seckey_slice(&secp, &key_bytes)?;
        let (internal_key, _parity) = XOnlyPublicKey::from_keypair(&keypair);

        let reveal_script = build_reveal_script(&internal_key, &payload);

        // Create taproot spend info
        let taproot_spend_info =
            create_taproot_spend_info(&secp, internal_key, reveal_script.clone())?;

        // Create taproot address for the commit output
        let taproot_address = Address::p2tr(
            &secp,
            internal_key,
            taproot_spend_info.merkle_root(),
            Network::Regtest,
        );

        // Fund the taproot address (commit transaction)
        let (commit_txid, commit_vout) = self
            .create_funding_utxo(&taproot_address, funding_amount)
            .await?;
        let commit_outpoint = OutPoint::new(commit_txid, commit_vout);

        // Build SPS-50 compliant OP_RETURN tag using TagData and ParseConfig
        let tag_data = TagDataRef::new(subprotocol_id, tx_type, &[])?;
        let parse_config = ParseConfig::new(self.params.magic_bytes);
        let op_return_script = parse_config.encode_script_buf(&tag_data)?;

        let op_return_output = TxOut {
            value: Amount::ZERO,
            script_pubkey: op_return_script,
        };

        // Change output
        let change_amount = funding_amount - fee;
        let change_address = self.client.get_new_address().await?;
        let change_output = TxOut {
            value: change_amount,
            script_pubkey: change_address.script_pubkey(),
        };

        // Create witness with control block and reveal script
        let control_block = taproot_spend_info
            .control_block(&(reveal_script.clone(), LeafVersion::TapScript))
            .ok_or_else(|| anyhow::anyhow!("Failed to create control block"))?;

        let mut witness = Witness::new();
        witness.push(reveal_script.as_bytes());
        witness.push(control_block.serialize());

        let tx_input = TxIn {
            previous_output: commit_outpoint,
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness,
        };

        let reveal_tx = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![tx_input],
            output: vec![op_return_output, change_output],
        };

        Ok(reveal_tx)
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Build envelope script for embedding data in Bitcoin transaction.
///
/// Creates an OP_FALSE OP_IF ... OP_ENDIF envelope containing the payload,
/// followed by OP_TRUE to satisfy tapscript's requirement that exactly one
/// value remains on the stack after execution.
fn build_envelope_script(payload: &[u8]) -> ScriptBuf {
    let mut builder = script::Builder::new()
        .push_opcode(OP_FALSE)
        .push_opcode(OP_IF);

    // Insert data in chunks (max 520 bytes per push)
    for chunk in payload.chunks(520) {
        builder = builder.push_slice(PushBytesBuf::try_from(chunk.to_vec()).unwrap());
    }

    builder = builder.push_opcode(OP_ENDIF);

    // Tapscript requires exactly one element on stack after execution
    // OP_TRUE (OP_1) leaves a single TRUE value
    builder = builder.push_int(1);

    builder.into_script()
}

/// Build taproot reveal script containing payload envelope.
///
/// Creates a taproot leaf script with just the envelope pattern.
/// In tapscript, we don't need OP_CHECKMULTISIG - the taproot control
/// block already proves the script is authorized.
fn build_reveal_script(_taproot_public_key: &XOnlyPublicKey, payload: &[u8]) -> ScriptBuf {
    // In tapscript, we only need the envelope - the control block proves authorization
    build_envelope_script(payload)
}

/// Create taproot spend info with reveal script.
///
/// Builds a taproot tree with the reveal script as a leaf.
fn create_taproot_spend_info(
    secp: &Secp256k1<All>,
    internal_key: XOnlyPublicKey,
    reveal_script: ScriptBuf,
) -> anyhow::Result<TaprootSpendInfo> {
    let taproot_spend_info = TaprootBuilder::new()
        .add_leaf(0, reveal_script)?
        .finalize(secp, internal_key)
        .map_err(|_| anyhow::anyhow!("Failed to finalize taproot spend info"))?;

    Ok(taproot_spend_info)
}

// ============================================================================
// Factory Function
// ============================================================================

/// Helper to create a test harness with default genesis height (101).
pub async fn create_test_harness() -> anyhow::Result<AsmTestHarness> {
    AsmTestHarness::new(101).await
}
