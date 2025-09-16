use strata_primitives::buf::Buf32;

/// Represents the state of the OL Chain
#[derive(Debug, Clone, Default)]
pub struct OLState {
    accounts_root: Buf32,
    l1_view: L1View,
    cur_slot: u64,
    cur_epoch: u64,
}

/// Represents the view of Layer 1 blockchain from the perspective of the OL
#[derive(Debug, Clone, Default)]
pub struct L1View {
    block_hash: Buf32,
    block_height: u64,
}

impl OLState {
    pub fn new(accounts_root: Buf32, l1_view: L1View, cur_slot: u64, cur_epoch: u64) -> Self {
        Self {
            accounts_root,
            l1_view,
            cur_slot,
            cur_epoch,
        }
    }

    pub fn accounts_root(&self) -> &Buf32 {
        &self.accounts_root
    }

    pub fn l1_view(&self) -> &L1View {
        &self.l1_view
    }

    pub fn cur_slot(&self) -> u64 {
        self.cur_slot
    }

    pub fn cur_epoch(&self) -> u64 {
        self.cur_epoch
    }

    pub fn set_accounts_root(&mut self, accounts_root: Buf32) {
        self.accounts_root = accounts_root;
    }

    pub fn set_l1_view(&mut self, l1_view: L1View) {
        self.l1_view = l1_view;
    }

    pub fn set_cur_slot(&mut self, cur_slot: u64) {
        self.cur_slot = cur_slot;
    }

    pub fn set_cur_epoch(&mut self, cur_epoch: u64) {
        self.cur_epoch = cur_epoch;
    }
}

impl L1View {
    pub fn new(block_hash: Buf32, block_height: u64) -> Self {
        Self {
            block_hash,
            block_height,
        }
    }

    pub fn block_hash(&self) -> &Buf32 {
        &self.block_hash
    }

    pub fn block_height(&self) -> u64 {
        self.block_height
    }

    pub fn set_block_hash(&mut self, block_hash: Buf32) {
        self.block_hash = block_hash;
    }

    pub fn set_block_height(&mut self, block_height: u64) {
        self.block_height = block_height;
    }
}
