//! Manages and updates unified status bundles
use std::sync::Arc;

use strata_primitives::l1::L1Status;
use strata_state::{
    bridge_state::{DepositsTable, OperatorTable},
    chain_state::Chainstate,
    client_state::{ClientState, L1Checkpoint, SyncState},
    id::L2BlockId,
};
use thiserror::Error;
use tokio::sync::watch::{self, error::RecvError};
use tracing::warn;

use crate::chain::*;

#[derive(Debug, Error)]
pub enum StatusError {
    #[error("not initialized yet")]
    NotInitialized,

    #[error("{0}")]
    Other(String),
}

/// A wrapper around the status sender and receiver.
///
/// This struct provides a convenient way to manage and access
/// both the sender and receiver components of a status communication channel.
// This structure is actually kinda problematic since it means that there's
// hidden dataflows that could be hard to reason about.  I am not sure of a
// better standalone solution at this time.
#[derive(Debug, Clone)]
pub struct StatusChannel {
    /// Shared reference to the status sender.
    sender: Arc<StatusSender>,
    /// Shared reference to the status receiver.
    receiver: Arc<StatusReceiver>,
}

impl StatusChannel {
    /// Creates a new `StatusChannel` for managing communication between components.
    ///
    /// # Arguments
    ///
    /// * `cl_state` - Initial state for the client.
    /// * `l1_status` - Initial L1 status.
    /// * `ch_state` - Optional initial FCM state.
    ///
    /// # Returns
    ///
    /// A `StatusChannel` containing a sender and receiver for the provided states.
    pub fn new(
        cl_state: ClientState,
        l1_status: L1Status,
        ch_state: Option<ChainSyncStatusUpdate>,
    ) -> Self {
        let (cl_tx, cl_rx) = watch::channel(cl_state);
        let (l1_tx, l1_rx) = watch::channel(l1_status);
        let (chs_tx, chs_rx) = watch::channel(ch_state);

        let sender = Arc::new(StatusSender {
            cl: cl_tx,
            l1: l1_tx,
            chs: chs_tx,
        });
        let receiver = Arc::new(StatusReceiver {
            cl: cl_rx,
            l1: l1_rx,
            chs: chs_rx,
        });

        Self { sender, receiver }
    }

    // Receiver methods

    /// Gets the last finalized [`L1Checkpoint`] from the current client state.
    pub fn get_last_checkpoint(&self) -> Option<L1Checkpoint> {
        self.receiver.cl.borrow().get_last_checkpoint().cloned()
    }

    /// Gets the latest [`SyncState`].
    #[deprecated(note = "use `.get_cur_sync_state()`")]
    pub fn sync_state(&self) -> Option<SyncState> {
        self.receiver.cl.borrow().sync().cloned()
    }

    /// Gets the latest [`SyncState`].
    pub fn get_cur_sync_state(&self) -> Option<SyncState> {
        self.receiver.cl.borrow().sync().cloned()
    }

    /// Returns a clone of the most recent tip block's chainstate, if present.
    pub fn get_cur_tip_chainstate(&self) -> Option<Arc<Chainstate>> {
        self.receiver
            .chs
            .borrow()
            .as_ref()
            .map(|css| css.new_tl_chainstate().clone())
    }

    pub fn get_cur_tip_chainstate_with_block(&self) -> Option<(Arc<Chainstate>, L2BlockId)> {
        self.receiver.chs.borrow().as_ref().map(|css| {
            (
                css.new_tl_chainstate().clone(),
                *css.new_status().tip_blkid(),
            )
        })
    }

    /// Gets the latest operator table.
    pub fn cur_tip_operator_table(&self) -> Option<OperatorTable> {
        self.get_cur_tip_chainstate()
            .map(|chs| chs.operator_table().clone())
    }

    /// Gets the latest deposits table.
    pub fn cur_tip_deposits_table(&self) -> Option<DepositsTable> {
        self.get_cur_tip_chainstate()
            .map(|chs| chs.deposits_table().clone())
    }

    /// Gets the latest [`L1Status`].
    #[deprecated(note = "use `.get_l1_status()`")]
    pub fn l1_status(&self) -> L1Status {
        self.receiver.l1.borrow().clone()
    }

    /// Gets the latest [`L1Status`].
    pub fn get_l1_status(&self) -> L1Status {
        self.receiver.l1.borrow().clone()
    }

    /// Gets the current chain tip epoch, if present.
    pub fn get_cur_chain_epoch(&self) -> Option<u64> {
        self.receiver
            .chs
            .borrow()
            .to_owned()
            .map(|ch| ch.cur_epoch())
    }

    #[deprecated(note = "use `.get_cur_tip_chainstate()`")]
    pub fn chain_state(&self) -> Option<Chainstate> {
        self.get_cur_tip_chainstate()
            .map(|chs| chs.as_ref().clone())
    }

    #[deprecated(note = "use `.get_cur_client_state()`")]
    pub fn client_state(&self) -> ClientState {
        self.receiver.cl.borrow().clone()
    }

    pub fn get_cur_client_state(&self) -> ClientState {
        self.receiver.cl.borrow().clone()
    }

    pub fn get_last_sync_status_update(&self) -> Option<ChainSyncStatusUpdate> {
        self.receiver.chs.borrow().clone()
    }

    /// Gets the chain sync status, which is regularly updated by the FCM
    /// whenever the tip changes, if set.
    pub fn get_chain_sync_status(&self) -> Option<ChainSyncStatus> {
        self.receiver
            .chs
            .borrow()
            .as_ref()
            .map(|chs| chs.new_status())
    }

    // Subscription functions.

    /// Create a subscription to the client state watcher.
    pub fn subscribe_client_state(&self) -> watch::Receiver<ClientState> {
        self.sender.cl.subscribe()
    }

    /// Create a subscription to the chain sync status watcher.
    pub fn subscribe_chain_sync(&self) -> watch::Receiver<Option<ChainSyncStatusUpdate>> {
        self.sender.chs.subscribe()
    }

    /// Waits until genesis and returns the client state where genesis was triggered.
    pub async fn wait_until_genesis(&self) -> Result<ClientState, RecvError> {
        let mut rx = self.subscribe_client_state();
        let state = rx.wait_for(|state| state.has_genesis_occurred()).await?;
        Ok(state.clone())
    }

    // Sender methods

    /// Sends the updated `Chainstate` to the chain state receiver. Logs a warning if the receiver
    /// is dropped.
    pub fn update_chain_sync_status(&self, update: ChainSyncStatusUpdate) {
        if self.sender.chs.send(Some(update)).is_err() {
            warn!("chain state receiver dropped");
        }
    }

    /// Sends the updated `ClientState` to the client state receiver. Logs a warning if the receiver
    /// is dropped.
    pub fn update_client_state(&self, post_state: ClientState) {
        if self.sender.cl.send(post_state).is_err() {
            warn!("client state receiver dropped");
        }
    }

    /// Sends the updated `L1Status` to the L1 status receiver. Logs a warning if the receiver is
    /// dropped.
    pub fn update_l1_status(&self, post_state: L1Status) {
        if self.sender.l1.send(post_state).is_err() {
            warn!("l1 status receiver dropped");
        }
    }
}

/// Wrapper for watch status receivers
#[derive(Debug, Clone)]
struct StatusReceiver {
    cl: watch::Receiver<ClientState>,
    l1: watch::Receiver<L1Status>,
    chs: watch::Receiver<Option<ChainSyncStatusUpdate>>,
}

/// Wrapper for watch status senders
#[derive(Debug, Clone)]
struct StatusSender {
    cl: watch::Sender<ClientState>,
    l1: watch::Sender<L1Status>,
    chs: watch::Sender<Option<ChainSyncStatusUpdate>>,
}
