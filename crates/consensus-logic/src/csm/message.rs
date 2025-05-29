use std::sync::Arc;

use strata_state::{client_state::ClientState, id::L2BlockId};

/// Sync control message.
#[derive(Copy, Clone, Debug)]
pub enum CsmMessage {
    /// Process a sync event at a given index.
    EventInput(u64),
}

/// Message about a new block the fork choice manager might do something with.
#[derive(Clone, Debug)]
pub enum ForkChoiceMessage {
    /// New block coming in from over the network to be considered.
    NewBlock(L2BlockId),
}

/// Package describing a new consensus state produced from a new sync event.
#[derive(Clone, Debug)]
pub struct ClientUpdateNotif {
    sync_event_idx: u64,
    new_state: Arc<ClientState>,
}

impl ClientUpdateNotif {
    pub const fn new(sync_event_idx: u64, new_state: Arc<ClientState>) -> Self {
        Self {
            sync_event_idx,
            new_state,
        }
    }

    pub const fn sync_event_idx(&self) -> u64 {
        self.sync_event_idx
    }

    #[expect(clippy::missing_const_for_fn)]
    pub fn new_state(&self) -> &ClientState {
        &self.new_state
    }
}
