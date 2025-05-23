//! Loads and formats Strata receipt RPC response.

use reth_node_api::{FullNodeComponents, NodeTypes};
use reth_primitives::{Receipt, TransactionMeta, TransactionSigned};
use reth_provider::{ReceiptProvider, TransactionsProvider};
use reth_rpc_eth_api::{helpers::LoadReceipt, FromEthApiError, RpcReceipt};
use reth_rpc_eth_types::{EthApiError, EthReceiptBuilder};

use crate::StrataEthApi;

impl<N> LoadReceipt for StrataEthApi<N>
where
    Self: Send + Sync,
    N: FullNodeComponents<Types: NodeTypes<ChainSpec = reth_chainspec::ChainSpec>>,
    Self::Provider:
        TransactionsProvider<Transaction = TransactionSigned> + ReceiptProvider<Receipt = Receipt>,
{
    async fn build_transaction_receipt(
        &self,
        tx: TransactionSigned,
        meta: TransactionMeta,
        receipt: Receipt,
    ) -> Result<RpcReceipt<Self::NetworkTypes>, Self::Error> {
        let hash = meta.block_hash;
        // get all receipts for the block
        let all_receipts = self
            .inner
            .eth_api
            .cache()
            .get_receipts(hash)
            .await
            .map_err(Self::Error::from_eth_err)?
            .ok_or(EthApiError::HeaderNotFound(hash.into()))?;

        // TODO: fix blob params
        Ok(EthReceiptBuilder::new(&tx, meta, &receipt, &all_receipts, None)?.build())
    }
}
