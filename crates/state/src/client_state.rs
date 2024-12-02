//! Consensus types that track node behavior as we receive messages from the L1
//! chain and the p2p network.  These will be expanded further as we actually
//! implement the consensus logic.
// TODO move this to another crate that contains our sync logic

use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_primitives::buf::Buf32;

use crate::{
    batch::{BatchInfo, BootstrapState},
    id::L2BlockId,
    l1::{HeaderVerificationState, L1BlockId},
};

/// High level client's state of the network.  This is local to the client, not
/// coordinated as part of the L2 chain.
///
/// This is updated when we see a consensus-relevant message.  This is L2 blocks
/// but also L1 blocks being published with relevant things in them, and
/// various other events.
#[derive(
    Clone, Debug, Eq, PartialEq, Arbitrary, BorshSerialize, BorshDeserialize, Deserialize, Serialize,
)]
pub struct ClientState {
    /// If we are after genesis.
    pub(super) chain_active: bool,

    /// State of the client tracking a genesised chain, after knowing about a
    /// valid chain.
    pub(super) sync_state: Option<SyncState>,

    /// Local view of the L1 state that we compare against the chain's view of
    /// L1 state.
    pub(super) local_l1_view: LocalL1State,

    /// L1 block we start watching the chain from.  We can't access anything
    /// before this chain height.
    pub(super) horizon_l1_height: u64,

    /// Height at which we'll create the L2 genesis block from.
    pub(super) genesis_l1_height: u64,

    /// Hash of verification state at `genesis_l1_height`. The hash is computed via
    /// [`super::l1::HeaderVerificationState::compute_hash`]
    pub(super) genesis_l1_verification_state_hash: Option<Buf32>,
}

impl ClientState {
    /// Creates the basic genesis client state from the genesis parameters.
    // TODO do we need this or should we load it at run time from the rollup params?
    pub fn from_genesis_params(horizon_l1_height: u64, genesis_l1_height: u64) -> Self {
        Self {
            chain_active: false,
            sync_state: None,
            local_l1_view: LocalL1State::new(horizon_l1_height),
            horizon_l1_height,
            genesis_l1_height,
            genesis_l1_verification_state_hash: None,
        }
    }

    /// If the chain is "active", meaning we are after genesis (although we
    /// don't necessarily know what it is, that's dictated by the `SyncState`).
    pub fn is_chain_active(&self) -> bool {
        self.chain_active
    }

    /// Returns a ref to the inner sync state, if it exists.
    pub fn sync(&self) -> Option<&SyncState> {
        self.sync_state.as_ref()
    }

    pub fn has_genesis_occurred(&self) -> bool {
        self.chain_active && self.sync().is_some()
    }

    /// Returns a ref to the local L1 view.
    pub fn l1_view(&self) -> &LocalL1State {
        &self.local_l1_view
    }

    pub fn l1_view_mut(&mut self) -> &mut LocalL1State {
        &mut self.local_l1_view
    }

    /// Overwrites the sync state.
    pub fn set_sync_state(&mut self, ss: SyncState) {
        self.sync_state = Some(ss);
    }

    /// Returns a mut ref to the inner sync state.  Only valid if we've observed
    /// genesis.  Only meant to be called when applying sync writes.
    pub fn expect_sync_mut(&mut self) -> &mut SyncState {
        self.sync_state
            .as_mut()
            .expect("clientstate: missing sync state")
    }

    pub fn most_recent_l1_block(&self) -> Option<&L1BlockId> {
        self.local_l1_view.local_unaccepted_blocks.last()
    }

    pub fn next_exp_l1_block(&self) -> u64 {
        self.local_l1_view.next_expected_block
    }

    pub fn genesis_l1_height(&self) -> u64 {
        self.genesis_l1_height
    }

    pub fn genesis_verification_hash(&self) -> Option<Buf32> {
        self.genesis_l1_verification_state_hash
    }
}

/// Relates to our view of the L2 chain, does not exist before genesis.
// TODO maybe include tip height and finalized height?  or their headers?
#[derive(
    Clone, Debug, Eq, PartialEq, Arbitrary, BorshDeserialize, BorshSerialize, Deserialize, Serialize,
)]
pub struct SyncState {
    /// Height of last L2 block we've chosen as the current tip.
    pub(super) tip_height: u64,

    /// Last L2 block we've chosen as the current tip.
    pub(super) tip_blkid: L2BlockId,

    /// L2 checkpoint blocks that have been confirmed on L1 and proven along with L1 block height.
    /// These are ordered by height
    pub(super) confirmed_checkpoint_blocks: Vec<(L1BlockHeight, L2BlockId)>,

    /// L2 block that's been finalized on L1 and proven
    pub(super) finalized_blkid: L2BlockId,
}

type L1BlockHeight = u64;

impl SyncState {
    pub fn from_genesis_blkid(gblkid: L2BlockId) -> Self {
        Self {
            tip_height: 0,
            tip_blkid: gblkid,
            confirmed_checkpoint_blocks: Vec::new(),
            finalized_blkid: gblkid,
        }
    }

    pub fn chain_tip_blkid(&self) -> &L2BlockId {
        &self.tip_blkid
    }

    pub fn finalized_blkid(&self) -> &L2BlockId {
        &self.finalized_blkid
    }

    pub fn confirmed_checkpoint_blocks(&self) -> &[(u64, L2BlockId)] {
        &self.confirmed_checkpoint_blocks
    }

    /// See if there's a checkpoint block at given l1_height
    pub fn get_confirmed_checkpt_block_at(&self, l1_height: u64) -> Option<L2BlockId> {
        self.confirmed_checkpoint_blocks
            .iter()
            .find(|(h, _)| *h == l1_height)
            .map(|e| e.1)
    }

    pub fn chain_tip_height(&self) -> u64 {
        self.tip_height
    }
}

#[derive(
    Clone, Debug, Eq, PartialEq, Arbitrary, BorshDeserialize, BorshSerialize, Deserialize, Serialize,
)]
pub struct LocalL1State {
    /// Local sequence of blocks that should reorg blocks in the chainstate.
    ///
    /// This MUST be ordered by block height, so the first block here is the
    /// buried height +1.
    // TODO this needs more tracking to make it remember where we are properly
    pub(super) local_unaccepted_blocks: Vec<L1BlockId>,

    /// Next L1 block height we expect to receive
    pub(super) next_expected_block: u64,

    /// Last finalized checkpoint
    pub(super) last_finalized_checkpoint: Option<L1Checkpoint>,

    /// Checkpoints that are in L1 but yet to be finalized.
    pub(super) verified_checkpoints: Vec<L1Checkpoint>,

    /// This state is used to verify the `next_expected_block`
    pub(super) header_verification_state: Option<HeaderVerificationState>,
}

impl LocalL1State {
    /// Constructs a new instance of the local L1 state bookkeeping.
    ///
    /// # Panics
    ///
    /// If we try to construct it in a way that implies we don't have the L1 genesis block.
    pub fn new(next_expected_block: u64) -> Self {
        if next_expected_block == 0 {
            panic!("clientstate: tried to construct without known L1 genesis block");
        }

        Self {
            local_unaccepted_blocks: Vec::new(),
            next_expected_block,
            verified_checkpoints: Vec::new(),
            last_finalized_checkpoint: None,
            header_verification_state: None,
        }
    }

    /// Returns a slice of the unaccepted blocks.
    pub fn local_unaccepted_blocks(&self) -> &[L1BlockId] {
        &self.local_unaccepted_blocks
    }

    /// Returns the height of the next block we expected to receive.
    pub fn next_expected_block(&self) -> u64 {
        self.next_expected_block
    }

    /// Returned the height of the buried L1 block, which we can't reorg to.
    pub fn buried_l1_height(&self) -> u64 {
        self.next_expected_block - self.local_unaccepted_blocks.len() as u64
    }

    /// Returns an iterator over the unaccepted L2 blocks, from the lowest up.
    pub fn unacc_blocks_iter(&self) -> impl Iterator<Item = (u64, &L1BlockId)> {
        self.local_unaccepted_blocks()
            .iter()
            .enumerate()
            .map(|(i, b)| (self.buried_l1_height() + i as u64, b))
    }

    pub fn tip_height(&self) -> u64 {
        if self.next_expected_block == 0 {
            panic!("clientstate: started without L1 genesis block somehow");
        }

        self.next_expected_block - 1
    }

    pub fn tip_blkid(&self) -> Option<&L1BlockId> {
        self.local_unaccepted_blocks().last()
    }

    pub fn last_finalized_checkpoint(&self) -> Option<&L1Checkpoint> {
        self.last_finalized_checkpoint.as_ref()
    }

    pub fn verified_checkpoints(&self) -> &[L1Checkpoint] {
        &self.verified_checkpoints
    }

    pub fn has_verified_checkpoint_before(&self, height: u64) -> bool {
        self.verified_checkpoints
            .iter()
            .any(|cp| cp.height <= height)
    }

    pub fn get_last_verified_checkpoint_before(&self, height: u64) -> Option<&L1Checkpoint> {
        self.verified_checkpoints
            .iter()
            .take_while(|cp| cp.height <= height)
            .last()
    }

    pub fn tip_verification_state(&self) -> Option<&HeaderVerificationState> {
        self.header_verification_state.as_ref()
    }

    pub fn get_verified_l1_height(&self, block_height: u64) -> Option<u64> {
        self.verified_checkpoints.last().and_then(|ch| {
            if ch.batch_info.includes_l2_block(block_height) {
                Some(ch.height)
            } else {
                None
            }
        })
    }
}

#[derive(
    Clone, Debug, Eq, PartialEq, Arbitrary, BorshDeserialize, BorshSerialize, Deserialize, Serialize,
)]
pub struct L1Checkpoint {
    /// The inner checkpoint batch info
    pub batch_info: BatchInfo,

    /// The inner checkpoint batch info
    // TODO why is this called bootstrap state?
    pub bootstrap_state: BootstrapState,

    /// If the checkpoint included proof
    pub is_proved: bool,

    /// L1 block height it appears in
    pub height: u64,
}

impl L1Checkpoint {
    pub fn new(
        batch_info: BatchInfo,
        bootstrap_state: BootstrapState,
        is_proved: bool,
        height: u64,
    ) -> Self {
        Self {
            batch_info,
            bootstrap_state,
            is_proved,
            height,
        }
    }
}
