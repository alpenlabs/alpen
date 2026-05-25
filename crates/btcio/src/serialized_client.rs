//! Serialized access wrapper for Bitcoin RPC clients.

use std::sync::Arc;

use bitcoin::{bip32::Xpriv, block::Header, Address, Block, BlockHash, Network, Transaction, Txid};
use bitcoind_async_client::{
    corepc_types::{
        model::{
            GetAddressInfo, GetBlockchainInfo, GetMempoolInfo, GetRawMempool, GetRawMempoolVerbose,
            GetRawTransaction, GetRawTransactionVerbose, GetTransaction, GetTxOut,
            ListTransactions, ListUnspent, PsbtBumpFee, SignRawTransactionWithWallet,
            SubmitPackage, TestMempoolAccept, WalletCreateFundedPsbt, WalletProcessPsbt,
        },
        v29::ImportDescriptors,
    },
    traits::{Broadcaster, Reader, Signer, Wallet},
    types::{
        CreateRawTransactionArguments, CreateRawTransactionInput, CreateRawTransactionOutput,
        ImportDescriptorInput, ListUnspentQueryOptions, PreviousTransactionOutput,
        PsbtBumpFeeOptions, SighashType, WalletCreateFundedPsbtOptions,
    },
    ClientResult,
};
use tokio::sync::Mutex;

use crate::broadcaster::{BroadcasterError, TxLookupOutcome, WalletTxLookup};

/// Wraps a Bitcoin RPC client so only one request uses its HTTP pool at a time.
#[derive(Clone, Debug)]
pub struct SerializedBitcoinClient<T> {
    inner: Arc<Mutex<T>>,
}

impl<T> SerializedBitcoinClient<T> {
    /// Creates a serialized wrapper around `inner`.
    pub fn new(inner: T) -> Self {
        Self {
            inner: Arc::new(Mutex::new(inner)),
        }
    }
}

impl<T> Reader for SerializedBitcoinClient<T>
where
    T: Reader + Send,
{
    async fn estimate_smart_fee(&self, conf_target: u16) -> ClientResult<u64> {
        let client = self.inner.lock().await;
        client.estimate_smart_fee(conf_target).await
    }

    async fn get_block_header(&self, hash: &BlockHash) -> ClientResult<Header> {
        let hash = *hash;
        let client = self.inner.lock().await;
        client.get_block_header(&hash).await
    }

    async fn get_block(&self, hash: &BlockHash) -> ClientResult<Block> {
        let hash = *hash;
        let client = self.inner.lock().await;
        client.get_block(&hash).await
    }

    async fn get_block_height(&self, hash: &BlockHash) -> ClientResult<u64> {
        let hash = *hash;
        let client = self.inner.lock().await;
        client.get_block_height(&hash).await
    }

    async fn get_block_header_at(&self, height: u64) -> ClientResult<Header> {
        let client = self.inner.lock().await;
        client.get_block_header_at(height).await
    }

    async fn get_block_at(&self, height: u64) -> ClientResult<Block> {
        let client = self.inner.lock().await;
        client.get_block_at(height).await
    }

    async fn get_block_count(&self) -> ClientResult<u64> {
        let client = self.inner.lock().await;
        client.get_block_count().await
    }

    async fn get_block_hash(&self, height: u64) -> ClientResult<BlockHash> {
        let client = self.inner.lock().await;
        client.get_block_hash(height).await
    }

    async fn get_blockchain_info(&self) -> ClientResult<GetBlockchainInfo> {
        let client = self.inner.lock().await;
        client.get_blockchain_info().await
    }

    async fn get_current_timestamp(&self) -> ClientResult<u32> {
        let client = self.inner.lock().await;
        client.get_current_timestamp().await
    }

    async fn get_raw_mempool(&self) -> ClientResult<GetRawMempool> {
        let client = self.inner.lock().await;
        client.get_raw_mempool().await
    }

    async fn get_raw_mempool_verbose(&self) -> ClientResult<GetRawMempoolVerbose> {
        let client = self.inner.lock().await;
        client.get_raw_mempool_verbose().await
    }

    async fn get_mempool_info(&self) -> ClientResult<GetMempoolInfo> {
        let client = self.inner.lock().await;
        client.get_mempool_info().await
    }

    async fn get_raw_transaction_verbosity_zero(
        &self,
        txid: &Txid,
    ) -> ClientResult<GetRawTransaction> {
        let txid = *txid;
        let client = self.inner.lock().await;
        client.get_raw_transaction_verbosity_zero(&txid).await
    }

    async fn get_raw_transaction_verbosity_one(
        &self,
        txid: &Txid,
    ) -> ClientResult<GetRawTransactionVerbose> {
        let txid = *txid;
        let client = self.inner.lock().await;
        client.get_raw_transaction_verbosity_one(&txid).await
    }

    async fn get_tx_out(
        &self,
        txid: &Txid,
        vout: u32,
        include_mempool: bool,
    ) -> ClientResult<GetTxOut> {
        let txid = *txid;
        let client = self.inner.lock().await;
        client.get_tx_out(&txid, vout, include_mempool).await
    }

    async fn network(&self) -> ClientResult<Network> {
        let client = self.inner.lock().await;
        client.network().await
    }
}

impl<T> Broadcaster for SerializedBitcoinClient<T>
where
    T: Broadcaster + Send,
{
    async fn send_raw_transaction(&self, tx: &Transaction) -> ClientResult<Txid> {
        let tx = tx.clone();
        let client = self.inner.lock().await;
        client.send_raw_transaction(&tx).await
    }

    async fn test_mempool_accept(&self, tx: &Transaction) -> ClientResult<TestMempoolAccept> {
        let tx = tx.clone();
        let client = self.inner.lock().await;
        client.test_mempool_accept(&tx).await
    }

    async fn submit_package(&self, txs: &[Transaction]) -> ClientResult<SubmitPackage> {
        let txs = txs.to_vec();
        let client = self.inner.lock().await;
        client.submit_package(&txs).await
    }
}

impl<T> Wallet for SerializedBitcoinClient<T>
where
    T: Wallet + Send,
{
    async fn get_new_address(&self) -> ClientResult<Address> {
        let client = self.inner.lock().await;
        client.get_new_address().await
    }

    async fn get_transaction(&self, txid: &Txid) -> ClientResult<GetTransaction> {
        let txid = *txid;
        let client = self.inner.lock().await;
        client.get_transaction(&txid).await
    }

    async fn list_transactions(&self, count: Option<usize>) -> ClientResult<ListTransactions> {
        let client = self.inner.lock().await;
        client.list_transactions(count).await
    }

    async fn list_wallets(&self) -> ClientResult<Vec<String>> {
        let client = self.inner.lock().await;
        client.list_wallets().await
    }

    async fn create_raw_transaction(
        &self,
        raw_tx: CreateRawTransactionArguments,
    ) -> ClientResult<Transaction> {
        let client = self.inner.lock().await;
        client.create_raw_transaction(raw_tx).await
    }

    async fn wallet_create_funded_psbt(
        &self,
        inputs: &[CreateRawTransactionInput],
        outputs: &[CreateRawTransactionOutput],
        locktime: Option<u32>,
        options: Option<WalletCreateFundedPsbtOptions>,
        bip32_derivs: Option<bool>,
    ) -> ClientResult<WalletCreateFundedPsbt> {
        let inputs = inputs.to_vec();
        let outputs = outputs.to_vec();
        let client = self.inner.lock().await;
        client
            .wallet_create_funded_psbt(&inputs, &outputs, locktime, options, bip32_derivs)
            .await
    }

    async fn get_address_info(&self, address: &Address) -> ClientResult<GetAddressInfo> {
        let address = address.clone();
        let client = self.inner.lock().await;
        client.get_address_info(&address).await
    }

    async fn list_unspent(
        &self,
        min_conf: Option<u32>,
        max_conf: Option<u32>,
        addresses: Option<&[Address]>,
        include_unsafe: Option<bool>,
        query_options: Option<ListUnspentQueryOptions>,
    ) -> ClientResult<ListUnspent> {
        let addresses = addresses.map(<[Address]>::to_vec);
        let client = self.inner.lock().await;
        client
            .list_unspent(
                min_conf,
                max_conf,
                addresses.as_deref(),
                include_unsafe,
                query_options,
            )
            .await
    }
}

impl<T> Signer for SerializedBitcoinClient<T>
where
    T: Signer + Send,
{
    async fn sign_raw_transaction_with_wallet(
        &self,
        tx: &Transaction,
        prev_outputs: Option<Vec<PreviousTransactionOutput>>,
    ) -> ClientResult<SignRawTransactionWithWallet> {
        let tx = tx.clone();
        let client = self.inner.lock().await;
        client
            .sign_raw_transaction_with_wallet(&tx, prev_outputs)
            .await
    }

    async fn get_xpriv(&self) -> ClientResult<Option<Xpriv>> {
        let client = self.inner.lock().await;
        client.get_xpriv().await
    }

    async fn import_descriptors(
        &self,
        descriptors: Vec<ImportDescriptorInput>,
        wallet_name: String,
    ) -> ClientResult<ImportDescriptors> {
        let client = self.inner.lock().await;
        client.import_descriptors(descriptors, wallet_name).await
    }

    async fn wallet_process_psbt(
        &self,
        psbt: &str,
        sign: Option<bool>,
        sighashtype: Option<SighashType>,
        bip32_derivs: Option<bool>,
    ) -> ClientResult<WalletProcessPsbt> {
        let psbt = psbt.to_owned();
        let client = self.inner.lock().await;
        client
            .wallet_process_psbt(&psbt, sign, sighashtype, bip32_derivs)
            .await
    }

    async fn psbt_bump_fee(
        &self,
        txid: &Txid,
        options: Option<PsbtBumpFeeOptions>,
    ) -> ClientResult<PsbtBumpFee> {
        let txid = *txid;
        let client = self.inner.lock().await;
        client.psbt_bump_fee(&txid, options).await
    }
}

impl<T> WalletTxLookup for SerializedBitcoinClient<T>
where
    T: WalletTxLookup + Send,
{
    async fn get_transaction_confirmation(
        &self,
        txid: &Txid,
    ) -> Result<TxLookupOutcome, BroadcasterError> {
        let txid = *txid;
        let client = self.inner.lock().await;
        client.get_transaction_confirmation(&txid).await
    }
}
