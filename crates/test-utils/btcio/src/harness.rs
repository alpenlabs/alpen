use std::collections::HashMap;

use bitcoin::{Address, BlockHash, OutPoint, Transaction, Txid, constants::COINBASE_MATURITY};
use bitcoind_async_client::Client;
use corepc_node::Node;
use strata_crypto::{EvenSecretKey, test_utils::schnorr::Musig2Tweak};

use crate::{client::get_bitcoind_and_client, mining, submit, transaction, utils::block_on};

/// High level test harness that wraps a regtest `bitcoind` node and RPC client.
///
/// The harness makes it easy to spin up an isolated node, mine blocks for funding,
/// and submit transactions using the helper functions in this crate.
#[derive(Debug)]
pub struct BtcioTestHarness {
    bitcoind: Node,
    client: Client,
}

impl BtcioTestHarness {
    /// Start a fresh regtest node and RPC client.
    pub fn new() -> Self {
        let (bitcoind, client) = get_bitcoind_and_client();
        Self { bitcoind, client }
    }

    /// Start a new harness and immediately mine `count` blocks so the wallet has spendable funds.
    pub fn new_with_mined_blocks(count: usize, address: Option<Address>) -> anyhow::Result<Self> {
        let harness = Self::new();
        harness.mine_blocks_blocking(count, address)?;
        Ok(harness)
    }

    /// Convenience helper that mines enough blocks to exceed coinbase maturity.
    pub fn new_with_coinbase_maturity() -> anyhow::Result<Self> {
        Self::new_with_mined_blocks((COINBASE_MATURITY + 1) as usize, None)
    }

    /// Accessor for the underlying node when low level calls are required.
    pub fn bitcoind(&self) -> &Node {
        &self.bitcoind
    }

    /// Accessor for the RPC client.
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Consume the harness and return the owned node and client.
    pub fn into_parts(self) -> (Node, Client) {
        (self.bitcoind, self.client)
    }

    /// Mine blocks asynchronously.
    pub async fn mine_blocks(
        &self,
        count: usize,
        address: Option<Address>,
    ) -> anyhow::Result<Vec<BlockHash>> {
        mining::mine_blocks(&self.bitcoind, &self.client, count, address).await
    }

    /// Mine blocks synchronously.
    pub fn mine_blocks_blocking(
        &self,
        count: usize,
        address: Option<Address>,
    ) -> anyhow::Result<Vec<BlockHash>> {
        mining::mine_blocks_blocking(&self.bitcoind, &self.client, count, address)
    }

    /// Submit and sign a transaction using a single key synchronously.
    pub fn submit_transaction_with_key_blocking(
        &self,
        secret_key: &EvenSecretKey,
        tx: &mut Transaction,
    ) -> anyhow::Result<Txid> {
        block_on(submit::submit_transaction_with_key(
            &self.bitcoind,
            &self.client,
            secret_key,
            tx,
        ))
    }

    /// Submit and sign a transaction using MuSig2 aggregation synchronously.
    pub fn submit_transaction_with_keys_blocking(
        &self,
        secret_keys: &[EvenSecretKey],
        tx: &mut Transaction,
        input_tweaks: Option<&HashMap<OutPoint, Musig2Tweak>>,
    ) -> anyhow::Result<Txid> {
        block_on(submit::submit_transaction_with_keys(
            &self.bitcoind,
            &self.client,
            secret_keys,
            tx,
            input_tweaks,
        ))
    }

    /// Broadcast a fully signed transaction.
    pub fn broadcast_transaction(&self, tx: &Transaction) -> anyhow::Result<Txid> {
        transaction::broadcast_transaction(&self.bitcoind, tx)
    }
}

impl Default for BtcioTestHarness {
    fn default() -> Self {
        BtcioTestHarness::new()
    }
}
