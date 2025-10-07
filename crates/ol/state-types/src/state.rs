use strata_primitives::buf::Buf32;

/// The Orchestration Layer(OL) State.
#[derive(Debug, Clone)]
pub struct OLState {
    accounts_root: Buf32,
    l1_view: L1View,
    cur_slot: u64,
    cur_epoch: u64,
    l1_recorded_epoch: EpochCommitment,
    total_btc_bridged: u64, // sats
}

impl OLState {
    pub fn new(
        accounts_root: Buf32,
        l1_view: L1View,
        cur_slot: u64,
        cur_epoch: u64,
        l1_recorded_epoch: EpochCommitment,
        total_btc_bridged: u64,
    ) -> Self {
        Self {
            accounts_root,
            l1_view,
            cur_slot,
            cur_epoch,
            l1_recorded_epoch,
            total_btc_bridged,
        }
    }

    pub fn accounts_root(&self) -> Buf32 {
        self.accounts_root
    }

    pub fn l1_view_mut(&self) -> &L1View {
        &self.l1_view
    }

    pub fn cur_slot_mut(&self) -> u64 {
        self.cur_slot
    }

    pub fn cur_epoch(&self) -> u64 {
        self.cur_epoch
    }

    pub fn l1_recorded_epoch(&self) -> &EpochCommitment {
        &self.l1_recorded_epoch
    }

    pub fn total_btc_bridged(&self) -> u64 {
        self.total_btc_bridged
    }
}

/// View of L1 as seen by OL.
#[derive(Debug, Clone)]
pub struct L1View {
    block_hash: Buf32,
    block_height: u64,
    // TODO: add witness root mmr
}

impl L1View {
    pub fn new(block_hash: Buf32, block_height: u64) -> Self {
        Self {
            block_hash,
            block_height,
        }
    }

    pub fn block_hash(&self) -> Buf32 {
        self.block_hash
    }

    pub fn block_height(&self) -> u64 {
        self.block_height
    }
}

/// Commitment to an epoch.
#[derive(Debug, Clone)]
pub struct EpochCommitment {
    /// Epoch number.
    epoch: u64,
    /// State root at the end of the epoch.
    state_root: Buf32,
    /// Terminal slot.
    terminal_slot: u64,
    /// Terminal block id.
    terminal_blockid: Buf32,
}

impl EpochCommitment {
    pub fn new(epoch: u64, state_root: Buf32, terminal_slot: u64, terminal_blockid: Buf32) -> Self {
        Self {
            epoch,
            state_root,
            terminal_slot,
            terminal_blockid,
        }
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn state_root(&self) -> Buf32 {
        self.state_root
    }

    pub fn terminal_slot(&self) -> u64 {
        self.terminal_slot
    }

    pub fn terminal_blockid(&self) -> Buf32 {
        self.terminal_blockid
    }
}
