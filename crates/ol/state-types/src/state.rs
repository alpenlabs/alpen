use strata_ol_chain_types_new::EpochCommitment;
use strata_primitives::{buf::Buf32, l1::BitcoinAmount};

/// The Orchestration Layer(OL) State.
#[derive(Debug, Clone)]
pub struct OLState {
    /// The root of the accounts tree which is stored outside this toplevel state.
    accounts_root: Buf32,

    /// The OL's view of L1.
    l1_view: L1View,

    /// The current slot.
    cur_slot: u64,

    /// The current epoch.
    cur_epoch: u64,

    /// Total deposits into OL.
    total_btc_bridged: BitcoinAmount,
}

impl OLState {
    pub fn new(
        accounts_root: Buf32,
        l1_view: L1View,
        cur_slot: u64,
        cur_epoch: u64,
        total_btc_bridged: BitcoinAmount,
    ) -> Self {
        Self {
            accounts_root,
            l1_view,
            cur_slot,
            cur_epoch,
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

    pub fn total_btc_bridged(&self) -> BitcoinAmount {
        self.total_btc_bridged
    }
}

/// View of L1 as seen by OL.
#[derive(Debug, Clone)]
pub struct L1View {
    /// Latest seen block id.
    block_id: Buf32,

    /// Latest seen block height.
    block_height: u64,

    /// Latest seen checkpoint corresponding to an epoch.
    recorded_epoch: EpochCommitment,
    // TODO add witnesses/manifests root mmr?
}

impl L1View {
    pub fn new(block_id: Buf32, block_height: u64, recorded_epoch: EpochCommitment) -> Self {
        Self {
            block_id,
            block_height,
            recorded_epoch,
        }
    }

    pub fn block_id(&self) -> Buf32 {
        self.block_id
    }

    pub fn recorded_epoch(&self) -> &EpochCommitment {
        &self.recorded_epoch
    }

    pub fn block_height(&self) -> u64 {
        self.block_height
    }
}
