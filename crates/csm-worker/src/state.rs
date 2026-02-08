//! CSM worker service state.

use std::sync::Arc;
#[cfg(test)]
use std::sync::Mutex;

use bitcoind_async_client::{client::Client as BitcoinClient, traits::Reader};
use strata_csm_types::ClientState;
use strata_identifiers::Epoch;
use strata_params::Params;
use strata_primitives::{
    l1::{BitcoinTxid, RawBitcoinTx},
    prelude::*,
};
use strata_service::ServiceState;
use strata_status::StatusChannel;
use strata_storage::NodeStorage;
use tokio::runtime::Handle;

use crate::constants;

/// State for the CSM worker service.
///
/// This state is used by the CSM worker which acts as a listener to ASM worker
/// status updates, processing checkpoint logs from the checkpoint-v0 subprotocol.
#[expect(
    missing_debug_implementations,
    reason = "NodeStorage doesn't implement Debug"
)]
pub struct CsmWorkerState {
    /// Consensus parameters.
    pub(crate) _params: Arc<Params>,

    /// Node storage handle.
    pub(crate) storage: Arc<NodeStorage>,

    /// Current client state.
    pub(crate) cur_state: Arc<ClientState>,

    /// Last ASM update we processed.
    pub(crate) last_asm_block: Option<L1BlockCommitment>,

    /// Last epoch we processed a checkpoint for.
    pub(crate) last_processed_epoch: Option<Epoch>,

    /// Runtime handle used for blocking RPC calls from this sync worker.
    pub(crate) runtime_handle: Option<Handle>,

    /// Bitcoin reader client shared with BTCIO reader task.
    pub(crate) bitcoin_client: Option<Arc<BitcoinClient>>,

    /// Enables resolving checkpoint pre-state from legacy `l2` block storage.
    // TODO: remove this once we delete the "old" code and functional tests
    pub(crate) use_legacy_l2_pre_state: bool,

    #[cfg(test)]
    /// Test-only raw tx fixtures keyed by txid.
    pub(crate) checkpoint_txs: Mutex<Vec<(BitcoinTxid, RawBitcoinTx)>>,

    /// Status channel for publishing state updates.
    pub(crate) status_channel: Arc<StatusChannel>,
}

impl CsmWorkerState {
    /// Create a new CSM worker state.
    pub fn new(
        params: Arc<Params>,
        storage: Arc<NodeStorage>,
        runtime_handle: Handle,
        bitcoin_client: Arc<BitcoinClient>,
        // TODO: remove this once we delete the "old" code and functional tests
        use_legacy_l2_pre_state: bool,
        status_channel: Arc<StatusChannel>,
    ) -> anyhow::Result<Self> {
        // Load the most recent client state from storage
        let (cur_block, cur_state) = storage
            .client_state()
            .fetch_most_recent_state()?
            .unwrap_or((params.rollup.genesis_l1_view.blk, ClientState::default()));

        Ok(Self {
            _params: params,
            storage,
            cur_state: Arc::new(cur_state),
            last_asm_block: Some(cur_block),
            last_processed_epoch: None,
            runtime_handle: Some(runtime_handle),
            bitcoin_client: Some(bitcoin_client),
            // TODO: remove this once we delete the "old" code and functional tests
            use_legacy_l2_pre_state,
            #[cfg(test)]
            checkpoint_txs: Mutex::new(Vec::new()),
            status_channel,
        })
    }

    #[cfg(test)]
    /// Create a test state that resolves checkpoint transactions from in-memory fixtures.
    pub(crate) fn new_for_tests(
        params: Arc<Params>,
        storage: Arc<NodeStorage>,
        status_channel: Arc<StatusChannel>,
    ) -> anyhow::Result<Self> {
        // Load the most recent client state from storage
        let (cur_block, cur_state) = storage
            .client_state()
            .fetch_most_recent_state()?
            .unwrap_or((params.rollup.genesis_l1_view.blk, ClientState::default()));

        Ok(Self {
            _params: params,
            storage,
            cur_state: Arc::new(cur_state),
            last_asm_block: Some(cur_block),
            last_processed_epoch: None,
            runtime_handle: None,
            bitcoin_client: None,
            // TODO: remove this once we delete the "old" code and functional tests
            use_legacy_l2_pre_state: false,
            checkpoint_txs: Mutex::new(Vec::new()),
            status_channel,
        })
    }

    /// Fetches a raw Bitcoin transaction by txid via the shared Bitcoin reader client.
    pub fn get_bitcoin_tx(&self, txid: &BitcoinTxid) -> anyhow::Result<RawBitcoinTx> {
        #[cfg(test)]
        if let Some(raw_tx) = self
            .checkpoint_txs
            .lock()
            .expect("checkpoint tx fixtures lock")
            .iter()
            .find_map(|(stored_txid, tx)| (stored_txid == txid).then(|| tx.clone()))
        {
            return Ok(raw_tx);
        }

        let runtime_handle = self
            .runtime_handle
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("runtime handle missing in CSM worker state"))?;
        let bitcoin_client = self
            .bitcoin_client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("bitcoin client missing in CSM worker state"))?;

        let raw_tx = runtime_handle
            .block_on(bitcoin_client.get_raw_transaction_verbosity_zero(&txid.inner()))
            .map_err(|e| anyhow::anyhow!("failed to fetch checkpoint tx {:?}: {e:?}", txid))?
            .0;
        Ok(RawBitcoinTx::from(raw_tx))
    }

    #[cfg(test)]
    /// Inserts a checkpoint tx fixture that [`Self::get_bitcoin_tx`] resolves before RPC.
    pub(crate) fn insert_checkpoint_tx_fixture(&self, txid: BitcoinTxid, raw_tx: RawBitcoinTx) {
        self.checkpoint_txs
            .lock()
            .expect("checkpoint tx fixtures lock")
            .push((txid, raw_tx));
    }

    /// Get the last ASM block that was processed.
    pub fn last_asm_block(&self) -> Option<L1BlockCommitment> {
        self.last_asm_block
    }
}

impl ServiceState for CsmWorkerState {
    fn name(&self) -> &str {
        constants::SERVICE_NAME
    }
}
