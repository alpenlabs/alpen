//! Serialized access wrapper for Bitcoin RPC clients.

use std::{fmt, sync::Arc};

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
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::broadcaster::{BroadcasterError, TxLookupOutcome, WalletTxLookup};

/// Wraps a Bitcoin RPC client so only one request uses its HTTP pool at a time.
///
/// Alpen's EE sequencer shares one wallet-backed Bitcoin RPC client between
/// the L1 broadcaster and chunked-envelope DA writer. The upstream client has
/// a fixed-size HTTP pool and no public concurrency knob, so this wrapper keeps
/// wallet and broadcast RPC calls serialized at the call boundary.
#[derive(Clone)]
pub struct SerializedBitcoinClient<T> {
    inner: T,
    request_permit: Arc<Semaphore>,
}

impl<T: fmt::Debug> fmt::Debug for SerializedBitcoinClient<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SerializedBitcoinClient")
            .field("inner", &self.inner)
            .finish_non_exhaustive()
    }
}

impl<T> SerializedBitcoinClient<T> {
    /// Creates a serialized wrapper around `inner`.
    pub fn new(inner: T) -> Self {
        Self {
            inner,
            request_permit: Arc::new(Semaphore::new(1)),
        }
    }

    async fn acquire_request_permit(&self) -> OwnedSemaphorePermit {
        self.request_permit
            .clone()
            .acquire_owned()
            .await
            .expect("request permit is never closed")
    }
}

impl<T> Reader for SerializedBitcoinClient<T>
where
    T: Reader + Send + Sync,
{
    async fn estimate_smart_fee(&self, conf_target: u16) -> ClientResult<u64> {
        let _request_permit = self.acquire_request_permit().await;
        self.inner.estimate_smart_fee(conf_target).await
    }

    async fn get_block_header(&self, hash: &BlockHash) -> ClientResult<Header> {
        let hash = *hash;
        let _request_permit = self.acquire_request_permit().await;
        self.inner.get_block_header(&hash).await
    }

    async fn get_block(&self, hash: &BlockHash) -> ClientResult<Block> {
        let hash = *hash;
        let _request_permit = self.acquire_request_permit().await;
        self.inner.get_block(&hash).await
    }

    async fn get_block_height(&self, hash: &BlockHash) -> ClientResult<u64> {
        let hash = *hash;
        let _request_permit = self.acquire_request_permit().await;
        self.inner.get_block_height(&hash).await
    }

    async fn get_block_header_at(&self, height: u64) -> ClientResult<Header> {
        let _request_permit = self.acquire_request_permit().await;
        self.inner.get_block_header_at(height).await
    }

    async fn get_block_at(&self, height: u64) -> ClientResult<Block> {
        let _request_permit = self.acquire_request_permit().await;
        self.inner.get_block_at(height).await
    }

    async fn get_block_count(&self) -> ClientResult<u64> {
        let _request_permit = self.acquire_request_permit().await;
        self.inner.get_block_count().await
    }

    async fn get_block_hash(&self, height: u64) -> ClientResult<BlockHash> {
        let _request_permit = self.acquire_request_permit().await;
        self.inner.get_block_hash(height).await
    }

    async fn get_blockchain_info(&self) -> ClientResult<GetBlockchainInfo> {
        let _request_permit = self.acquire_request_permit().await;
        self.inner.get_blockchain_info().await
    }

    async fn get_current_timestamp(&self) -> ClientResult<u32> {
        let _request_permit = self.acquire_request_permit().await;
        self.inner.get_current_timestamp().await
    }

    async fn get_raw_mempool(&self) -> ClientResult<GetRawMempool> {
        let _request_permit = self.acquire_request_permit().await;
        self.inner.get_raw_mempool().await
    }

    async fn get_raw_mempool_verbose(&self) -> ClientResult<GetRawMempoolVerbose> {
        let _request_permit = self.acquire_request_permit().await;
        self.inner.get_raw_mempool_verbose().await
    }

    async fn get_mempool_info(&self) -> ClientResult<GetMempoolInfo> {
        let _request_permit = self.acquire_request_permit().await;
        self.inner.get_mempool_info().await
    }

    async fn get_raw_transaction_verbosity_zero(
        &self,
        txid: &Txid,
    ) -> ClientResult<GetRawTransaction> {
        let txid = *txid;
        let _request_permit = self.acquire_request_permit().await;
        self.inner.get_raw_transaction_verbosity_zero(&txid).await
    }

    async fn get_raw_transaction_verbosity_one(
        &self,
        txid: &Txid,
    ) -> ClientResult<GetRawTransactionVerbose> {
        let txid = *txid;
        let _request_permit = self.acquire_request_permit().await;
        self.inner.get_raw_transaction_verbosity_one(&txid).await
    }

    async fn get_tx_out(
        &self,
        txid: &Txid,
        vout: u32,
        include_mempool: bool,
    ) -> ClientResult<GetTxOut> {
        let txid = *txid;
        let _request_permit = self.acquire_request_permit().await;
        self.inner.get_tx_out(&txid, vout, include_mempool).await
    }

    async fn network(&self) -> ClientResult<Network> {
        let _request_permit = self.acquire_request_permit().await;
        self.inner.network().await
    }
}

impl<T> Broadcaster for SerializedBitcoinClient<T>
where
    T: Broadcaster + Send + Sync,
{
    async fn send_raw_transaction(&self, tx: &Transaction) -> ClientResult<Txid> {
        let tx = tx.clone();
        let _request_permit = self.acquire_request_permit().await;
        self.inner.send_raw_transaction(&tx).await
    }

    async fn test_mempool_accept(&self, tx: &Transaction) -> ClientResult<TestMempoolAccept> {
        let tx = tx.clone();
        let _request_permit = self.acquire_request_permit().await;
        self.inner.test_mempool_accept(&tx).await
    }

    async fn submit_package(&self, txs: &[Transaction]) -> ClientResult<SubmitPackage> {
        let txs = txs.to_vec();
        let _request_permit = self.acquire_request_permit().await;
        self.inner.submit_package(&txs).await
    }
}

impl<T> Wallet for SerializedBitcoinClient<T>
where
    T: Wallet + Send + Sync,
{
    async fn get_new_address(&self) -> ClientResult<Address> {
        let _request_permit = self.acquire_request_permit().await;
        self.inner.get_new_address().await
    }

    async fn get_transaction(&self, txid: &Txid) -> ClientResult<GetTransaction> {
        let txid = *txid;
        let _request_permit = self.acquire_request_permit().await;
        self.inner.get_transaction(&txid).await
    }

    async fn list_transactions(&self, count: Option<usize>) -> ClientResult<ListTransactions> {
        let _request_permit = self.acquire_request_permit().await;
        self.inner.list_transactions(count).await
    }

    async fn list_wallets(&self) -> ClientResult<Vec<String>> {
        let _request_permit = self.acquire_request_permit().await;
        self.inner.list_wallets().await
    }

    async fn create_raw_transaction(
        &self,
        raw_tx: CreateRawTransactionArguments,
    ) -> ClientResult<Transaction> {
        let _request_permit = self.acquire_request_permit().await;
        self.inner.create_raw_transaction(raw_tx).await
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
        let _request_permit = self.acquire_request_permit().await;
        self.inner
            .wallet_create_funded_psbt(&inputs, &outputs, locktime, options, bip32_derivs)
            .await
    }

    async fn get_address_info(&self, address: &Address) -> ClientResult<GetAddressInfo> {
        let address = address.clone();
        let _request_permit = self.acquire_request_permit().await;
        self.inner.get_address_info(&address).await
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
        let _request_permit = self.acquire_request_permit().await;
        self.inner
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
    T: Signer + Send + Sync,
{
    async fn sign_raw_transaction_with_wallet(
        &self,
        tx: &Transaction,
        prev_outputs: Option<Vec<PreviousTransactionOutput>>,
    ) -> ClientResult<SignRawTransactionWithWallet> {
        let tx = tx.clone();
        let _request_permit = self.acquire_request_permit().await;
        self.inner
            .sign_raw_transaction_with_wallet(&tx, prev_outputs)
            .await
    }

    async fn get_xpriv(&self) -> ClientResult<Option<Xpriv>> {
        let _request_permit = self.acquire_request_permit().await;
        self.inner.get_xpriv().await
    }

    async fn import_descriptors(
        &self,
        descriptors: Vec<ImportDescriptorInput>,
        wallet_name: String,
    ) -> ClientResult<ImportDescriptors> {
        let _request_permit = self.acquire_request_permit().await;
        self.inner
            .import_descriptors(descriptors, wallet_name)
            .await
    }

    async fn wallet_process_psbt(
        &self,
        psbt: &str,
        sign: Option<bool>,
        sighashtype: Option<SighashType>,
        bip32_derivs: Option<bool>,
    ) -> ClientResult<WalletProcessPsbt> {
        let psbt = psbt.to_owned();
        let _request_permit = self.acquire_request_permit().await;
        self.inner
            .wallet_process_psbt(&psbt, sign, sighashtype, bip32_derivs)
            .await
    }

    async fn psbt_bump_fee(
        &self,
        txid: &Txid,
        options: Option<PsbtBumpFeeOptions>,
    ) -> ClientResult<PsbtBumpFee> {
        let txid = *txid;
        let _request_permit = self.acquire_request_permit().await;
        self.inner.psbt_bump_fee(&txid, options).await
    }
}

impl<T> WalletTxLookup for SerializedBitcoinClient<T>
where
    T: WalletTxLookup + Send + Sync,
{
    async fn get_transaction_confirmation(
        &self,
        txid: &Txid,
    ) -> Result<TxLookupOutcome, BroadcasterError> {
        let txid = *txid;
        let _request_permit = self.acquire_request_permit().await;
        self.inner.get_transaction_confirmation(&txid).await
    }
}

#[cfg(test)]
mod tests {
    use std::slice::from_ref;

    use bitcoin::{
        consensus::encode::deserialize_hex, hashes::Hash, Amount, BlockHash, Network, Txid,
    };
    use bitcoind_async_client::{
        traits::{Broadcaster, Reader, Signer, Wallet},
        types::{
            CreateRawTransactionArguments, CreateRawTransactionOutput, ListUnspentQueryOptions,
            WalletCreateFundedPsbtOptions,
        },
    };

    use super::*;
    use crate::test_utils::{TestBitcoinClient, SOME_TX};

    #[tokio::test]
    async fn delegates_bitcoin_rpc_traits_through_serialized_client() {
        let client = SerializedBitcoinClient::new(TestBitcoinClient::new(2));
        let tx: Transaction = deserialize_hex(SOME_TX).expect("test tx should decode");
        let txid = tx.compute_txid();
        let block_hash = BlockHash::all_zeros();

        assert_eq!(client.estimate_smart_fee(1).await.unwrap(), 3);
        client.get_block_header(&block_hash).await.unwrap();
        client.get_block(&block_hash).await.unwrap();
        assert_eq!(client.get_block_height(&block_hash).await.unwrap(), 100);
        client.get_block_header_at(100).await.unwrap();
        client.get_block_at(100).await.unwrap();
        assert_eq!(client.get_block_count().await.unwrap(), 100);
        client.get_block_hash(100).await.unwrap();
        assert_eq!(client.get_blockchain_info().await.unwrap().blocks, 100);
        assert_eq!(client.get_current_timestamp().await.unwrap(), 1_000);
        assert!(client.get_raw_mempool().await.unwrap().0.is_empty());
        assert!(client.get_raw_mempool_verbose().await.unwrap().0.is_empty());
        assert_eq!(client.get_mempool_info().await.unwrap().size, 0);
        client
            .get_raw_transaction_verbosity_zero(&txid)
            .await
            .unwrap();
        client
            .get_raw_transaction_verbosity_one(&txid)
            .await
            .unwrap();
        client.get_tx_out(&txid, 0, true).await.unwrap();
        assert_eq!(client.network().await.unwrap(), Network::Regtest);

        assert_eq!(
            client.send_raw_transaction(&tx).await.unwrap(),
            Txid::from_slice(&[1; 32]).unwrap()
        );
        client.test_mempool_accept(&tx).await.unwrap();
        client.submit_package(from_ref(&tx)).await.unwrap();

        let address = client.get_new_address().await.unwrap();
        assert_eq!(
            client.get_transaction(&txid).await.unwrap().confirmations,
            2
        );
        assert!(client
            .list_transactions(Some(1))
            .await
            .unwrap()
            .0
            .is_empty());
        assert!(client.list_wallets().await.unwrap().is_empty());
        client
            .create_raw_transaction(CreateRawTransactionArguments {
                inputs: vec![],
                outputs: vec![CreateRawTransactionOutput::Data {
                    data: "00".to_string(),
                }],
            })
            .await
            .unwrap();
        client
            .wallet_create_funded_psbt(
                &[],
                &[CreateRawTransactionOutput::Data {
                    data: "00".to_string(),
                }],
                None,
                Some(WalletCreateFundedPsbtOptions::default()),
                Some(false),
            )
            .await
            .unwrap();
        assert!(client.get_address_info(&address).await.unwrap().is_mine);
        let query_options = ListUnspentQueryOptions {
            minimum_amount: Some(Amount::from_sat(1)),
            maximum_amount: None,
            maximum_count: Some(1),
        };
        assert_eq!(
            client
                .list_unspent(
                    Some(1),
                    None,
                    Some(from_ref(&address)),
                    Some(true),
                    Some(query_options),
                )
                .await
                .unwrap()
                .0
                .len(),
            1
        );

        assert!(
            client
                .sign_raw_transaction_with_wallet(&tx, None)
                .await
                .unwrap()
                .complete
        );
        assert!(client.get_xpriv().await.unwrap().is_some());
        assert!(
            client
                .import_descriptors(vec![], "testwallet".to_string())
                .await
                .unwrap()
                .0[0]
                .success
        );
        assert!(
            client
                .wallet_process_psbt("70736274ff", Some(false), None, Some(false))
                .await
                .unwrap()
                .complete
        );
        client.psbt_bump_fee(&txid, None).await.unwrap();

        let coverage = client
            .get_transaction_confirmation(&txid)
            .await
            .expect("wallet lookup should succeed");
        let TxLookupOutcome::Found(info) = coverage else {
            panic!("expected found transaction");
        };
        assert_eq!(info.confirmations, 2);
        assert_eq!(info.block_height.unwrap(), 100);
    }
}
