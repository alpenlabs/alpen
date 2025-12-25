//! Test harness for running ASM worker as a service with Bitcoin regtest
//!
//! This module provides infrastructure to run integration tests with:
//! - Bitcoin regtest node
//! - ASM worker service running in background
//! - Automatic block submission and processing
//! - State query utilities

use std::{sync::Arc, time::Duration};

use bitcoin::{
    absolute::LockTime,
    blockdata::script,
    consensus::deserialize,
    key::UntweakedKeypair,
    opcodes::{
        all::{OP_ENDIF, OP_IF},
        OP_FALSE,
    },
    script::PushBytesBuf,
    secp256k1::Secp256k1,
    taproot::{LeafVersion, TaprootBuilder, TaprootSpendInfo},
    transaction::Version,
    Address, Amount, Block, BlockHash, Network, OutPoint, ScriptBuf, Sequence, Transaction, TxIn,
    TxOut, Txid, Witness, XOnlyPublicKey,
};
use bitcoind_async_client::{
    traits::{Reader, Wallet},
    Client,
};
use corepc_node::Node;
use rand::RngCore;
use strata_asm_worker::{AsmWorkerBuilder, AsmWorkerHandle, WorkerContext};
use strata_params::Params;
use strata_primitives::{buf::Buf32, l1::L1BlockCommitment};
use strata_state::{asm_state::AsmState, BlockSubmitter};
use strata_tasks::{TaskExecutor, TaskManager};
use tokio::time::sleep;

use super::asm::{get_genesis_l1_view, TestAsmWorkerContext};

/// Test harness that manages ASM worker service and Bitcoin regtest
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
    pub params: Arc<Params>,
    /// Task executor for spawning tasks
    pub executor: TaskExecutor,
    /// Genesis block height
    pub genesis_height: u64,
}

impl AsmTestHarness {
    /// Create a new test harness with ASM worker service
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
        println!(
            "Test harness initialized with genesis at height {} ({})",
            genesis_height, genesis_hash
        );

        // 3. Setup parameters
        let mut params = strata_test_utils_l2::gen_params();
        params.rollup.network = Network::Regtest;
        let genesis_view = get_genesis_l1_view(&client, &genesis_hash).await?;
        params.rollup.genesis_l1_view = genesis_view;
        let params = Arc::new(params);

        // 4. Create worker context
        let context = TestAsmWorkerContext::new((*client).clone());

        // 5. Create task executor
        let task_manager = TaskManager::new(tokio::runtime::Handle::current());
        let executor = task_manager.create_executor();

        // 6. Launch ASM worker service
        let asm_handle = AsmWorkerBuilder::new()
            .with_context(context.clone())
            .with_params(params.clone())
            .launch(&executor)?;

        println!("ASM worker service launched successfully");

        Ok(Self {
            bitcoind,
            client,
            asm_handle,
            context,
            params,
            executor,
            genesis_height,
        })
    }

    /// Mine a single block and submit it to ASM worker
    ///
    /// This will:
    /// 1. Mine a block with the given address (or a new one if None)
    /// 2. Fetch the full block from Bitcoin
    /// 3. Cache it in the context
    /// 4. Submit the block commitment to ASM worker
    /// 5. Wait briefly for processing
    ///
    /// # Returns
    /// The block hash of the mined block
    pub async fn mine_and_submit_block(
        &self,
        address: Option<bitcoin::Address>,
    ) -> anyhow::Result<BlockHash> {
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

        println!("Mined block at height {} ({})", height, block_hash);

        // Create L1BlockCommitment and submit to ASM worker
        let block_id = block_hash.into();
        let block_commitment = L1BlockCommitment::new(
            bitcoin::absolute::Height::from_consensus(height as u32)?,
            block_id,
        );

        // Use block_in_place to submit synchronously within async context
        tokio::task::block_in_place(|| self.asm_handle.submit_block(block_commitment))?;

        println!("Submitted block {} to ASM worker", block_hash);

        // Wait a bit for ASM worker to process
        sleep(Duration::from_millis(100)).await;

        Ok(block_hash)
    }

    /// Mine multiple blocks and submit them to ASM worker
    ///
    /// # Arguments
    /// * `count` - Number of blocks to mine
    ///
    /// # Returns
    /// Vector of block hashes
    pub async fn mine_and_submit_blocks(&self, count: usize) -> anyhow::Result<Vec<BlockHash>> {
        let mut hashes = Vec::new();
        for _ in 0..count {
            let hash = self.mine_and_submit_block(None).await?;
            hashes.push(hash);
        }
        Ok(hashes)
    }

    /// Submit a transaction to Bitcoin regtest mempool
    ///
    /// Note: The transaction must be valid and properly funded
    pub async fn submit_transaction(
        &self,
        tx: &bitcoin::Transaction,
    ) -> anyhow::Result<bitcoin::Txid> {
        let result = self.bitcoind.client.send_raw_transaction(tx)?;
        Ok(result.0.parse()?)
    }

    /// Get a block from the cache or Bitcoin
    pub async fn get_block(&self, block_hash: BlockHash) -> anyhow::Result<Block> {
        self.context.fetch_and_cache_block(block_hash).await
    }

    /// Wait a brief period for ASM worker to process submitted blocks
    ///
    /// This is a simple delay to allow asynchronous block processing to complete.
    /// For more precise waiting, use `wait_for_height()` or `wait_for_block()`.
    pub async fn wait_for_processing(&self) {
        sleep(Duration::from_millis(200)).await;
    }

    /// Wait for ASM state to advance beyond a given height
    ///
    /// Polls the ASM state until it processes a block at or above the target height,
    /// or times out after the specified duration.
    pub async fn wait_for_height(
        &self,
        target_height: u64,
        timeout: Duration,
    ) -> anyhow::Result<()> {
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() > timeout {
                anyhow::bail!("Timeout waiting for height {}", target_height);
            }

            if let Some((commitment, _state)) = self.context.get_latest_asm_state()? {
                let current_height = commitment.height().to_consensus_u32() as u64;
                if current_height >= target_height {
                    return Ok(());
                }
            }

            sleep(Duration::from_millis(50)).await;
        }
    }

    /// Wait for a specific block to be processed by ASM worker
    ///
    /// Polls until the ASM state for the given block exists, or times out.
    pub async fn wait_for_block(
        &self,
        blockid: &L1BlockCommitment,
        timeout: Duration,
    ) -> anyhow::Result<AsmState> {
        let start = std::time::Instant::now();
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

    /// Get the current chain tip height from ASM state
    ///
    /// Returns the height of the latest processed block, or an error if no state exists.
    pub async fn get_chain_tip(&self) -> anyhow::Result<u64> {
        Ok(self.client.get_blockchain_info().await?.blocks)
    }

    /// Get the latest ASM state from the worker context
    pub fn get_latest_asm_state(&self) -> anyhow::Result<Option<(L1BlockCommitment, AsmState)>> {
        Ok(self.context.get_latest_asm_state()?)
    }

    /// Get ASM state at a specific block
    pub fn get_asm_state_at(&self, blockid: &L1BlockCommitment) -> anyhow::Result<AsmState> {
        Ok(self.context.get_anchor_state(blockid)?)
    }

    /// Get the number of MMR leaves (manifest hashes) stored
    pub fn get_mmr_leaf_count(&self) -> usize {
        self.context.mmr_leaves.lock().unwrap().len()
    }

    /// Get a manifest hash by index
    pub fn get_manifest_hash(&self, index: u64) -> anyhow::Result<Option<Buf32>> {
        Ok(self.context.get_manifest_hash(index)?.map(Buf32::from))
    }

    /// Mine blocks to an address to create spendable coinbase outputs
    ///
    /// This creates mature coinbase outputs that can be used for funding transactions.
    /// Mines `count` blocks and returns the address that received the coinbase.
    ///
    /// Note: Coinbase outputs require 100 confirmations to be spendable.
    pub async fn mine_blocks_for_funding(&self, count: usize) -> anyhow::Result<Address> {
        let address = self.client.get_new_address().await?;
        strata_test_utils_btcio::mine_blocks(
            &self.bitcoind,
            &self.client,
            count,
            Some(address.clone()),
        )
        .await?;
        Ok(address)
    }

    /// Get wallet balance
    pub async fn get_balance(&self) -> anyhow::Result<Amount> {
        let balance_result = self.bitcoind.client.get_balance()?;
        Ok(Amount::from_btc(balance_result.0)?)
    }

    /// Create a funding UTXO for transaction building
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

        let hex_tx = self
            .client
            .get_raw_transaction_verbosity_zero(&funding_txid)
            .await?
            .0;
        let tx_bytes = hex::decode(&hex_tx)?;
        let funding_tx: Transaction = deserialize(&tx_bytes)?;

        let prev_vout = funding_tx
            .output
            .iter()
            .enumerate()
            .find(|(_, output)| output.script_pubkey == address.script_pubkey())
            .map(|(idx, _)| idx as u32)
            .ok_or_else(|| anyhow::anyhow!("Could not find output in funding transaction"))?;

        Ok((funding_txid, prev_vout))
    }

    /// Build a funded admin transaction with real UTXOs
    ///
    /// This creates a proper Bitcoin transaction that:
    /// 1. Spends a real UTXO (funded by mining blocks)
    /// 2. Contains the admin payload in envelope script format
    /// 3. Is properly signed and ready for submission
    ///
    /// # Arguments
    /// * `admin_payload` - Borsh-serialized admin action + signatures
    /// * `tx_type` - Admin transaction type (e.g., SEQUENCER_UPDATE_TX_TYPE)
    /// * `fee` - Transaction fee
    pub async fn build_funded_admin_tx(
        &self,
        admin_payload: Vec<u8>,
        tx_type: u8,
        fee: Amount,
    ) -> anyhow::Result<Transaction> {
        // Calculate funding amount needed (outputs + fee buffer)
        let dust_amount = Amount::from_sat(1000);
        let funding_amount = fee + dust_amount + Amount::from_sat(1000); // Extra buffer

        // Ensure we have funds available
        let balance = self.get_balance().await?;
        if balance < funding_amount {
            println!(
                "Insufficient balance ({} sats), mining blocks for funding...",
                balance.to_sat()
            );
            self.mine_blocks_for_funding(101).await?;
        }

        // STEP 1: Generate taproot keypair and build reveal script with admin payload
        let secp = Secp256k1::new();
        let mut rng = rand::thread_rng();
        let mut key_bytes = [0u8; 32];
        rng.fill_bytes(&mut key_bytes);
        let keypair = UntweakedKeypair::from_seckey_slice(&secp, &key_bytes)?;
        let (internal_key, _parity) = XOnlyPublicKey::from_keypair(&keypair);

        println!(
            "Building reveal script with admin payload ({} bytes)...",
            admin_payload.len()
        );
        let reveal_script = build_reveal_script(&internal_key, &admin_payload);

        // STEP 2: Create taproot spend info with reveal script
        let taproot_spend_info =
            create_taproot_spend_info(&secp, internal_key, reveal_script.clone())?;

        // STEP 3: Create taproot address for the commit output
        let taproot_address = Address::p2tr(
            &secp,
            internal_key,
            taproot_spend_info.merkle_root(),
            Network::Regtest,
        );

        // STEP 4: Fund the taproot address (commit transaction)
        println!("Creating taproot funding UTXO at {}...", taproot_address);
        let (commit_txid, commit_vout) = self
            .create_funding_utxo(&taproot_address, funding_amount)
            .await?;
        let commit_outpoint = OutPoint::new(commit_txid, commit_vout);
        println!("Created commit UTXO: {}:{}", commit_txid, commit_vout);

        // STEP 5: Build reveal transaction that spends the taproot output
        // Create SPS-50 compliant OP_RETURN tag
        const ADMIN_SUBPROTOCOL_ID: u8 = 0;
        let mut sps50_tag = Vec::with_capacity(6);
        sps50_tag.extend_from_slice(&self.params.rollup().magic_bytes); // 4 bytes
        sps50_tag.push(ADMIN_SUBPROTOCOL_ID); // 1 byte
        sps50_tag.push(tx_type); // 1 byte

        // Create outputs
        let op_return_output = TxOut {
            value: Amount::ZERO,
            script_pubkey: ScriptBuf::new_op_return(PushBytesBuf::try_from(sps50_tag)?),
        };

        // Calculate change amount
        let change_amount = funding_amount - fee;
        let change_address = self.client.get_new_address().await?;
        let change_output = TxOut {
            value: change_amount,
            script_pubkey: change_address.script_pubkey(),
        };

        // STEP 6: Create witness with control block and reveal script
        let control_block = taproot_spend_info
            .control_block(&(reveal_script.clone(), LeafVersion::TapScript))
            .ok_or_else(|| anyhow::anyhow!("Failed to create control block"))?;

        let mut witness = Witness::new();
        witness.push(reveal_script.as_bytes());
        witness.push(control_block.serialize());

        println!(
            "Created taproot witness (script: {} bytes, control block: {} bytes)",
            reveal_script.as_bytes().len(),
            control_block.serialize().len()
        );

        // Create input spending the taproot UTXO
        let tx_input = TxIn {
            previous_output: commit_outpoint,
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness,
        };

        // Build reveal transaction
        let reveal_tx = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![tx_input],
            output: vec![op_return_output, change_output],
        };

        println!(
            "Built reveal transaction (txid: {})",
            reveal_tx.compute_txid()
        );
        println!(
            "Admin payload embedded in witness ({} bytes)",
            admin_payload.len()
        );

        Ok(reveal_tx)
    }

    /// Submit an admin transaction to mempool and mine it into a block
    ///
    /// This is the end-to-end flow:
    /// 1. Submit transaction to mempool
    /// 2. Mine a block that includes the transaction
    /// 3. Submit block to ASM worker
    /// 4. Wait for processing
    ///
    /// # Returns
    /// The block hash containing the transaction
    pub async fn submit_and_mine_admin_tx(&self, tx: &Transaction) -> anyhow::Result<BlockHash> {
        // Submit transaction to mempool
        let txid = self.submit_transaction(tx).await?;
        println!("Submitted admin tx {} to mempool", txid);

        // Mine a block that includes this transaction
        // Bitcoin Core will automatically include mempool txs when mining
        let block_hash = self.mine_and_submit_block(None).await?;

        println!("Mined block {} containing admin tx", block_hash);

        Ok(block_hash)
    }
}

/// Build envelope script for embedding data in Bitcoin transaction
///
/// Creates an OP_FALSE OP_IF ... OP_ENDIF envelope containing the payload,
/// followed by OP_TRUE to satisfy tapscript's requirement that exactly one
/// value remains on the stack after execution.
///
/// # Arguments
/// * `payload` - Data to embed (e.g., borsh-serialized admin action)
///
/// # Returns
/// ScriptBuf containing the envelope
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

/// Build taproot reveal script containing admin payload envelope
///
/// Creates a taproot leaf script with just the envelope pattern.
/// In tapscript, we don't need OP_CHECKMULTISIG - the taproot control
/// block already proves the script is authorized.
///
/// # Arguments
/// * `_taproot_public_key` - X-only public key (unused, kept for API consistency)
/// * `payload` - Admin payload to embed
///
/// # Returns
/// ScriptBuf containing the reveal script (just the envelope)
fn build_reveal_script(_taproot_public_key: &XOnlyPublicKey, payload: &[u8]) -> ScriptBuf {
    // In tapscript, we only need the envelope - the control block proves authorization
    build_envelope_script(payload)
}

/// Create taproot spend info with reveal script
///
/// Builds a taproot tree with the reveal script as a leaf.
///
/// # Arguments
/// * `secp` - Secp256k1 context
/// * `internal_key` - Internal public key for taproot
/// * `reveal_script` - Script containing admin payload
///
/// # Returns
/// TaprootSpendInfo for creating taproot addresses and control blocks
fn create_taproot_spend_info(
    secp: &Secp256k1<bitcoin::secp256k1::All>,
    internal_key: XOnlyPublicKey,
    reveal_script: ScriptBuf,
) -> anyhow::Result<TaprootSpendInfo> {
    let taproot_spend_info = TaprootBuilder::new()
        .add_leaf(0, reveal_script)?
        .finalize(secp, internal_key)
        .map_err(|_| anyhow::anyhow!("Failed to finalize taproot spend info"))?;

    Ok(taproot_spend_info)
}

/// Helper to create a test harness with default genesis height (101)
pub async fn create_test_harness() -> anyhow::Result<AsmTestHarness> {
    AsmTestHarness::new(101).await
}
