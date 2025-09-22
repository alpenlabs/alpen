use strata_primitives::buf::Buf32;

/// Represents the state of the OL Chain
#[derive(Debug, Clone, Default)]
pub struct OLState {
    /// Root of the accounts ledger
    accounts_root: Buf32,
    /// L1 view of OL
    l1_view: L1View,
    /// Current slot this state corresponds to
    cur_slot: u64,
    /// Current epoch
    cur_epoch: u64,
    /// Epoch whose checkpoint is recorded by ASM
    recorded_epoch: u64,
    /// Total bridged-in btc
    total_bridged_in_sats: u64,
}

/// Represents the view of the layer 1 blockchain from the perspective of the OL
#[derive(Debug, Clone, Default)]
pub struct L1View {
    block_hash: Buf32,
    block_height: u64,
}

impl OLState {
    pub fn new(
        accounts_root: Buf32,
        l1_view: L1View,
        cur_slot: u64,
        cur_epoch: u64,
        recorded_epoch: u64,
        total_bridged_in_sats: u64,
    ) -> Self {
        Self {
            accounts_root,
            l1_view,
            cur_slot,
            cur_epoch,
            recorded_epoch,
            total_bridged_in_sats,
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

    pub fn recorded_epoch(&self) -> u64 {
        self.recorded_epoch
    }

    pub fn total_bridged_in_sats(&self) -> u64 {
        self.total_bridged_in_sats
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

    // NOTE: will be redundant with SSZ
    pub fn compute_root(&self) -> Buf32 {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();

        // Hash accounts root
        hasher.update(self.accounts_root.as_ref());

        // Hash L1 view components
        hasher.update(self.l1_view.block_hash.as_ref());
        hasher.update(self.l1_view.block_height.to_be_bytes());

        // Hash current slot and epoch
        hasher.update(self.cur_slot.to_be_bytes());
        hasher.update(self.cur_epoch.to_be_bytes());

        Buf32::new(hasher.finalize().into())
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
