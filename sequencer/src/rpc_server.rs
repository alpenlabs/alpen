#![allow(unused)]

use std::{borrow::BorrowMut, sync::Arc};

use async_trait::async_trait;
use jsonrpsee::{
    core::RpcResult,
    types::{ErrorObject, ErrorObjectOwned},
};
use reth_primitives::{Address, BlockId, BlockNumberOrTag, Bytes, B256, B64, U256, U64};
use reth_rpc_api::EthApiServer;
use reth_rpc_types::{
    state::StateOverride, AccessListWithGasUsed, AnyTransactionReceipt, BlockOverrides, Bundle,
    EIP1186AccountProofResponse, EthCallResponse, FeeHistory, Header, Index, RichBlock,
    StateContext, SyncInfo, SyncStatus, Transaction, TransactionRequest, Work,
};
use thiserror::Error;
use tokio::sync::{mpsc, oneshot, watch, Mutex, RwLock};

use alpen_express_btcio::writer::{utils::calculate_blob_hash, DaWriter};
use alpen_express_consensus_logic::sync_manager::SyncManager;

use alpen_express_db::traits::{ChainstateProvider, Database, L2DataProvider};
use alpen_express_db::traits::{L1DataProvider, SequencerDatabase};
use alpen_express_primitives::buf::Buf32;
use alpen_express_rpc_api::{AlpenAdminApiServer, AlpenApiServer, HexBytes};
use alpen_express_rpc_types::{
    BlockHeader, ClientStatus, DaBlob, DepositEntry, DepositState, ExecUpdate, L1Status,
    WithdrawalIntent,
};
use alpen_express_state::{
    block::{L2Block, L2BlockBundle}, chain_state::ChainState, client_state::ClientState, da_blob::{BlobDest, BlobIntent}, exec_update, header::{L2Header, SignedL2BlockHeader}, id::L2BlockId, l1::L1BlockId
};

use tracing::*;

use crate::rpc_server;

#[derive(Debug, Error)]
pub enum Error {
    /// Unsupported RPCs for express.  Some of these might need to be replaced
    /// with standard unsupported errors.
    #[error("unsupported RPC")]
    Unsupported,

    #[error("not yet implemented")]
    Unimplemented,

    #[error("client not started")]
    ClientNotStarted,

    #[error("missing L2 block {0:?}")]
    MissingL2Block(L2BlockId),

    #[error("sync state missing because genesis hasn't happened yet")]
    MissingSyncState,

    #[error("missing chainstate for index {0}")]
    MissingChainstate(u64),

    #[error("db: {0}")]
    Db(#[from] alpen_express_db::errors::DbError),

    #[error("blocking task '{0}' failed for unknown reason")]
    BlockingAbort(String),

    #[error("incorrect parameters: {0}")]
    IncorrectParameters(String),

    /// Generic internal error message.  If this is used often it should be made
    /// into its own error type.
    #[error("{0}")]
    Other(String),

    /// Generic internal error message with a payload value.  If this is used
    /// often it should be made into its own error type.
    #[error("{0} (+data)")]
    OtherEx(String, serde_json::Value),
}

impl Error {
    pub fn code(&self) -> i32 {
        match self {
            Self::Unsupported => -32600,
            Self::Unimplemented => -32601,
            Self::IncorrectParameters(_) => -32602,
            Self::MissingL2Block(_) => -32603,
            Self::MissingChainstate(_) => -32604,
            Self::Db(_) => -32605,
            Self::ClientNotStarted => -32606,
            Self::MissingSyncState => -32607,
            Self::BlockingAbort(_) => -32001,
            Self::Other(_) => -32000,
            Self::OtherEx(_, _) => -32000,
        }
    }
}

impl From<Error> for ErrorObjectOwned {
    fn from(val: Error) -> Self {
        let code = val.code();
        match val {
            Error::OtherEx(m, b) => ErrorObjectOwned::owned::<_>(code, m.to_string(), Some(b)),
            _ => ErrorObjectOwned::owned::<serde_json::Value>(code, format!("{}", val), None),
        }
    }
}

fn fetch_l2blk<D: Database + Sync + Send + 'static>(
    l2_prov: Arc<<D as Database>::L2Prov>,
    blkid: L2BlockId,
) -> Result<L2BlockBundle, Error>
where
    <D as Database>::L2Prov: Send + Sync + 'static,
{
    l2_prov
        .get_block_data(blkid)
        .map_err(Error::Db)?
        .ok_or(Error::MissingL2Block(blkid))
}

pub struct AlpenRpcImpl<D> {
    l1_status: Arc<RwLock<L1Status>>,
    database: Arc<D>,
    sync_manager: Arc<SyncManager>,
    stop_tx: Mutex<Option<oneshot::Sender<()>>>,
}

impl<D: Database + Sync + Send + 'static> AlpenRpcImpl<D> {
    pub fn new(
        l1_status: Arc<RwLock<L1Status>>,
        database: Arc<D>,
        sync_manager: Arc<SyncManager>,
        stop_tx: oneshot::Sender<()>,
    ) -> Self {
        Self {
            l1_status,
            database,
            sync_manager,
            stop_tx: Mutex::new(Some(stop_tx)),
        }
    }

    /// Gets a ref to the current client state as of the last update.
    async fn get_client_state(&self) -> Arc<ClientState> {
        let cs_rx = self.sync_manager.create_state_watch_sub();
        let cs = cs_rx.borrow();
        cs.clone()
    }

    /// Gets a clone of the current client state and fetches the chainstate that
    /// of the L2 block that it considers the tip state.
    async fn get_cur_states(&self) -> Result<(Arc<ClientState>, Option<Arc<ChainState>>), Error> {
        let cs = self.get_client_state().await;

        if cs.sync().is_none() {
            return Ok((cs, None));
        }

        let ss = cs.sync().unwrap();
        let tip_blkid = *ss.chain_tip_blkid();

        let db = self.database.clone();
        let chs = wait_blocking("load_chainstate", move || {
            // FIXME this is horrible, the sync state should have the block
            // number in it somewhere
            let l2_prov = db.l2_provider();
            let tip_block = l2_prov
                .get_block_data(tip_blkid)?
                .ok_or(Error::MissingL2Block(tip_blkid))?;
            let idx = tip_block.header().blockidx();

            let chs_prov = db.chainstate_provider();
            let toplevel_st = chs_prov
                .get_toplevel_state(idx)?
                .ok_or(Error::MissingChainstate(idx))?;

            Ok(Arc::new(toplevel_st))
        })
        .await?;

        Ok((cs, Some(chs)))
    }
}

fn conv_blk_header_to_rpc(blk_header: &impl L2Header) -> BlockHeader {
    BlockHeader {
        block_idx: blk_header.blockidx(),
        timestamp: blk_header.timestamp(),
        block_id: *blk_header.get_blockid().as_ref(),
        prev_block: *blk_header.parent().as_ref(),
        l1_segment_hash: *blk_header.l1_payload_hash().as_ref(),
        exec_segment_hash: *blk_header.exec_payload_hash().as_ref(),
        state_root: *blk_header.state_root().as_ref(),
    }
}

#[async_trait]
impl<D: Database + Send + Sync + 'static> AlpenApiServer for AlpenRpcImpl<D>
where
    <D as Database>::L1Prov: Send + Sync + 'static,
    <D as Database>::L2Prov: Send + Sync + 'static,
{
    async fn protocol_version(&self) -> RpcResult<u64> {
        Ok(1)
    }

    async fn stop(&self) -> RpcResult<()> {
        let mut opt = self.stop_tx.lock().await;
        if let Some(stop_tx) = opt.take() {
            if stop_tx.send(()).is_err() {
                warn!("tried to send stop signal, channel closed");
            }
        }
        Ok(())
    }

    async fn get_l1_status(&self) -> RpcResult<L1Status> {
        let l1_status = self.l1_status.read().await.clone();

        Ok(l1_status)
    }

    async fn get_l1_connection_status(&self) -> RpcResult<bool> {
        Ok(self.l1_status.read().await.bitcoin_rpc_connected)
    }

    async fn get_l1_block_hash(&self, height: u64) -> RpcResult<String> {
        let db = self.database.clone();
        let blk_manifest = wait_blocking("l1_block_manifest", move || {
            Ok(db
                .l1_provider()
                .get_block_manifest(height)
                .unwrap()
                .unwrap())
        })
        .await?;
        Ok(format!("{:?}", blk_manifest.block_hash()))
    }

    async fn get_client_status(&self) -> RpcResult<ClientStatus> {
        let state = self.get_client_state().await;

        let last_l1 = state.most_recent_l1_block().copied().unwrap_or_else(|| {
            // TODO figure out a better way to do this
            warn!("last L1 block not set in client state, returning zero");
            L1BlockId::from(Buf32::zero())
        });

        // Copy these out of the sync state, if they're there.
        let (chain_tip, finalized_blkid) = state
            .sync()
            .map(|ss| (*ss.chain_tip_blkid(), *ss.finalized_blkid()))
            .unwrap_or_default();

        // FIXME make this load from cache, and put the data we actually want
        // here in the client state
        // FIXME error handling
        let db = self.database.clone();
        let slot: u64 = wait_blocking("load_cur_block", move || {
            let l2_prov = db.l2_provider();
            l2_prov
                .get_block_data(chain_tip)
                .map(|b| b.map(|b| b.header().blockidx()).unwrap_or(u64::MAX))
                .map_err(Error::from)
        })
        .await?;

        Ok(ClientStatus {
            chain_tip: *chain_tip.as_ref(),
            chain_tip_slot: slot,
            finalized_blkid: *finalized_blkid.as_ref(),
            last_l1_block: *last_l1.as_ref(),
            buried_l1_height: state.l1_view().buried_l1_height(),
        })
    }

    async fn get_recent_blocks(&self, count: u64) -> RpcResult<Vec<BlockHeader>> {
        // FIXME: sync state should have a block number
        let cl_state = self.get_client_state().await;
        let tip_blkid = *cl_state
            .sync()
            .ok_or(Error::ClientNotStarted)?
            .chain_tip_blkid();
        let db = self.database.clone();

        if count > 1000 {
            return Err(Error::IncorrectParameters(
                "can't provide blocks for count greater than 1000".to_string(),
            )
            .into());
        }

        let blk_headers = wait_blocking("block_headers", move || {
            let l2_prov = db.l2_provider();
            let l2_blk = fetch_l2blk::<D>(l2_prov.clone(), tip_blkid)?;

            let mut parent_blk = *l2_blk.header().parent();

            let blk_headers: Result<Vec<BlockHeader>, Error> = (0..count)
                .rev()
                .map(|_| {
                    let l2_blk = fetch_l2blk::<D>(l2_prov.clone(), parent_blk)?;
                    parent_blk = *l2_blk.header().parent();

                    Ok(conv_blk_header_to_rpc(l2_blk.header()))
                })
                .collect();

            blk_headers
        })
        .await?;

        Ok(blk_headers)
    }

    async fn get_blocks_at_idx(&self, idx: u64) -> RpcResult<Vec<BlockHeader>> {
        let cl_state = self.get_client_state().await;
        let tip_blkid = *cl_state
            .sync()
            .ok_or(Error::ClientNotStarted)?
            .chain_tip_blkid();
        let db = self.database.clone();

        let blk_header = wait_blocking("block_at_idx", move || {
            let l2_prov = db.l2_provider();
            // check the tip idx
            let tip_idx = fetch_l2blk::<D>(l2_prov.clone(), tip_blkid)?
                .header()
                .blockidx();

            if idx > tip_idx {
                return Err(Error::IncorrectParameters(format!(
                    "index {} greater than the tip_index {}",
                    idx, tip_idx
                )));
            }

            Ok(l2_prov
                .get_blocks_at_height(idx)
                .map_err(Error::Db)?
                .iter()
                .map(|blkid| {
                    let l2_blk = fetch_l2blk::<D>(l2_prov.clone(), *blkid)?;

                    Ok(conv_blk_header_to_rpc(l2_blk.block().header()))
                })
                .collect::<Result<Vec<BlockHeader>, Error>>())
        })
        .await?;

        Ok(blk_header?)
    }

    async fn get_block_by_id(
        &self,
        blkid: alpen_express_rpc_types::L2BlockId,
    ) -> RpcResult<BlockHeader> {
        let db = self.database.clone();
        let blkid = L2BlockId::from(Buf32::from(blkid.0));

        let l2_blk = wait_blocking("fetch_block", move || {
            let l2_prov = db.l2_provider();

            fetch_l2blk::<D>(l2_prov.clone(), blkid)
        })
        .await?;

        Ok(conv_blk_header_to_rpc(l2_blk.header()))
    }

    async fn get_exec_update_by_id(
        &self,
        blkid: alpen_express_rpc_types::L2BlockId,
    ) -> RpcResult<ExecUpdate> {
        let db = self.database.clone();
        let blkid = L2BlockId::from(Buf32::from(blkid.0));

        let l2_blk = wait_blocking("fetch_block", move || {
            let l2_prov = db.l2_provider();

            fetch_l2blk::<D>(l2_prov.clone(), blkid)
        })
        .await?;

        let exec_update = l2_blk.exec_segment().update();

        let withdrawals = exec_update
            .output()
            .withdrawals()
            .iter()
            .map(|intent| {
                let (amt, dest_pk) = intent.into_parts();
                let dest_pk = *dest_pk.as_ref();
                WithdrawalIntent { amt, dest_pk }
            })
            .collect();

        let da_blobs = exec_update
            .output()
            .da_blobs()
            .iter()
            .map(|blob| DaBlob {
                dest: blob.dest().into(),
                blob_commitment: *blob.commitment().as_ref(),
            })
            .collect();

        Ok(ExecUpdate {
            update_idx: exec_update.input().update_idx(),
            entries_root: *exec_update.input().entries_root().as_ref(),
            extra_payload: exec_update.input().extra_payload().to_vec(),
            new_state: *exec_update.output().new_state().as_ref(),
            withdrawals,
            da_blobs,
        })
    }

    async fn get_current_deposits(&self) -> RpcResult<Vec<u32>> {
        let (_, chain_state) = self.get_cur_states().await?;
        let chain_state = chain_state.ok_or(Error::MissingSyncState)?;

        Ok(chain_state
            .deposit_table()
            .get_all_deposits_ids_iter()
            .collect())
    }

    async fn get_current_deposit_by_id(&self, deposit_id: u32) -> RpcResult<DepositEntry> {
        let (_, chain_state) = self.get_cur_states().await?;
        let chain_state = chain_state.ok_or(Error::MissingSyncState)?;

        let deposit_entry = chain_state
            .deposit_table()
            .get_deposit(deposit_id)
            .ok_or(Error::Other("unknown deposit idx".to_string()))?;

        let state = match deposit_entry.deposit_state() {
            alpen_express_state::bridge_state::DepositState::Created(_) => DepositState::Created,
            alpen_express_state::bridge_state::DepositState::Accepted(_) => DepositState::Accepted,
            alpen_express_state::bridge_state::DepositState::Dispatched(_) => {
                DepositState::Dispatched
            }
            alpen_express_state::bridge_state::DepositState::Executed => DepositState::Executed,
        };

        Ok(DepositEntry {
            deposit_idx: deposit_id,
            amt: deposit_entry.amt(),
            state,
        })
    }
}
/// Wrapper around [``tokio::task::spawn_blocking``] that handles errors in th{ unimplemented!() }
/// external task and merges the errors into the standard RPC error type.
async fn wait_blocking<F, R>(name: &'static str, f: F) -> Result<R, Error>
where
    F: Fn() -> Result<R, Error> + Sync + Send + 'static,
    R: Sync + Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(v) => v,
        Err(_) => {
            error!(%name, "background task aborted for unknown reason");
            Err(Error::BlockingAbort(name.to_owned()))
        }
    }
}

pub struct AdminServerImpl<S> {
    pub writer: Arc<DaWriter<S>>,
}

impl<S: SequencerDatabase> AdminServerImpl<S> {
    pub fn new(writer: Arc<DaWriter<S>>) -> Self {
        Self { writer }
    }
}

#[async_trait]
impl<S: SequencerDatabase + Send + Sync + 'static> AlpenAdminApiServer for AdminServerImpl<S> {
    async fn submit_da_blob(&self, blob: HexBytes) -> RpcResult<()> {
        let commitment = calculate_blob_hash(&blob.0);
        let blobintent = BlobIntent::new(BlobDest::L1, commitment, blob.0);
        // NOTE: It would be nice to return reveal txid from the submit method. But creation of txs
        // is deferred to signer in the writer module
        if let Err(e) = self.writer.submit_intent_async(blobintent).await {
            return Err(Error::Other("".to_string()).into());
        }
        Ok(())
    }
}


fn parse_l2block_id(block_id: &str) -> Result<L2BlockId, Error> {
    if block_id.len() != 64 {
        return Err(Error::IncorrectParameters("invalid BlockId".into()));
    }

    let mut bytes = [0u8; 32];

    for (idx, byte) in bytes.iter_mut().enumerate().take(32) {
        let start = idx * 2;
        let chunk = &block_id[start..start + 2];
        let byte_str = std::str::from_utf8(chunk.as_bytes())
            .map_err(|_| Error::IncorrectParameters("invalid BlockId".to_string()))?;
        *byte = u8::from_str_radix(byte_str, 16)
            .map_err(|_| Error::IncorrectParameters("invalid BlockId".to_string()))?;
    }

    Ok(L2BlockId::from(Buf32::from(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_l2_blockid() {
        let block_id =
            parse_l2block_id("698567f5fc38ab42831e0455dc097094a7899b9480411ed73fbca99244b743c6");
        assert!(block_id.is_ok());

        // incomplete block id
        let block_id =
            parse_l2block_id("698567f5fc38ab42831e0455dc097094a7899b9480411ed73fbca99244b743");
        assert!(block_id.is_err());
    }
}
