//! Status RPC implementation for alpen-client

use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use alpen_ee_exec_chain::ExecChainHandle;
use alpen_ee_ol_tracker::OLTrackerHandle;
use async_trait::async_trait;
use jsonrpsee::{core::RpcResult, proc_macros::rpc};
use serde::{Deserialize, Serialize};
use strata_primitives::{Buf32, OLBlockCommitment};
use strata_rpc_utils::to_jsonrpsee_error;
use tokio::sync::RwLock;

// TODO: move to some more relevant crate
/// Status RPC API for alpen-client
#[cfg_attr(not(test), rpc(server, namespace = "alpen"))]
#[cfg_attr(test, rpc(server, client, namespace = "alpen"))]
pub trait AlpenClientStatusApi {
    /// Get execution chain status (only available for sequencer nodes)
    #[method(name = "getExecChainStatus")]
    async fn get_exec_chain_status(&self) -> RpcResult<Option<ExecChainStatus>>;

    /// Get OL tracking status
    #[method(name = "getOLTrackingStatus")]
    async fn get_ol_tracking_status(&self) -> RpcResult<OLTrackingStatus>;

    /// Get client health status
    #[method(name = "getHealth")]
    async fn get_health(&self) -> RpcResult<ClientHealth>;
}

/// Execution chain status
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExecChainStatus {
    /// Current chain tip
    pub tip: BlockInfo,
    // TODO: add other as needed
}

/// Block information
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BlockInfo {
    pub hash: Buf32,
    pub number: u64,
}

/// OL tracking status
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OLTrackingStatus {
    finalized_ol_block: OLBlockCommitment,
    confirmed_blkid: Buf32,
    finalized_blkid: Buf32,
}

/// Client health status
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClientHealth {
    /// Whether client is ready to serve requests
    pub is_ready: bool,
    /// Whether execution chain is available
    pub exec_chain_available: bool,
    /// Whether OL tracker is available
    pub ol_tracker_available: bool,
    /// Current timestamp
    pub timestamp: u64,
}

/// Status RPC implementation for alpen-client
pub(crate) struct AlpenClientStatusRpc {
    /// OL tracker handle
    ol_tracker: Option<OLTrackerHandle>,
    /// Exec chain handle (can be set later for sequencer nodes)
    exec_chain: Arc<RwLock<Option<ExecChainHandle>>>,
}

impl AlpenClientStatusRpc {
    /// Create new status RPC instance
    pub(crate) fn new(
        ol_tracker: Option<OLTrackerHandle>,
        exec_chain: Arc<RwLock<Option<ExecChainHandle>>>,
    ) -> Self {
        Self {
            ol_tracker,
            exec_chain,
        }
    }
}

#[async_trait]
impl AlpenClientStatusApiServer for AlpenClientStatusRpc {
    async fn get_exec_chain_status(&self) -> RpcResult<Option<ExecChainStatus>> {
        // Read the exec chain handle from the RwLock
        let exec_chain_guard = self.exec_chain.read().await;

        // Only available if exec chain handle has been set (sequencer nodes)
        if let Some(exec_chain) = exec_chain_guard.as_ref() {
            // Get tip information
            let tip_blocknumhash = exec_chain
                .get_best_block()
                .await
                .map_err(to_jsonrpsee_error("Failed to get tip block"))?;

            Ok(Some(ExecChainStatus {
                tip: BlockInfo {
                    hash: tip_blocknumhash.blockhash(),
                    number: tip_blocknumhash.blocknum(),
                },
            }))
        } else {
            // Non-sequencer nodes or handle not yet set
            Ok(None)
        }
    }

    async fn get_ol_tracking_status(&self) -> RpcResult<OLTrackingStatus> {
        let ol_tracker = self
            .ol_tracker
            .as_ref()
            .ok_or_else(|| to_jsonrpsee_error("OL tracker handle not available")(""))?;

        // Get consensus watcher for confirmed/finalized states
        let consensus_state = ol_tracker.consensus_watcher();
        let current_state = consensus_state.borrow().clone();
        let confirmed_blkid = *current_state.confirmed();
        let finalized_blkid = *current_state.finalized();
        let finalized_ol_block = ol_tracker.ol_status_watcher().borrow().ol_block;

        Ok(OLTrackingStatus {
            finalized_ol_block,
            confirmed_blkid,
            finalized_blkid,
        })
    }

    async fn get_health(&self) -> RpcResult<ClientHealth> {
        // Get current timestamp
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Check if exec chain is available
        let exec_chain_guard = self.exec_chain.read().await;
        let exec_chain_available = exec_chain_guard.is_some();
        drop(exec_chain_guard); // Release the lock early

        let ol_tracker_available = self.ol_tracker.is_some();

        // Client is ready if both core components are available
        let is_ready = ol_tracker_available; // OL tracker is minimum requirement

        Ok(ClientHealth {
            is_ready,
            exec_chain_available,
            ol_tracker_available,
            timestamp,
        })
    }
}
