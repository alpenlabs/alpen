//! Consensus types that track node behavior as we receive messages from the L1
//! chain and the p2p network.  These will be expanded further as we actually
//! implement the consensus logic.
// TODO move this to another crate that contains our sync logic

use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_primitives::{
    batch::BatchTransition, buf::Buf32, epoch::EpochCommitment, l1::L1BlockCommitment,
    params::Params,
};

use crate::{
    batch::BatchInfo,
    id::L2BlockId,
    l1::L1BlockId,
    operation::{ClientUpdateOutput, SyncAction},
    state_queue::StateQueue,
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
    /// State of the client tracking a genesised chain, after knowing about a
    /// valid chain.
    pub(super) genesis_blockid: Option<L2BlockId>,

    /// Height at which we'll create the L2 genesis block from.
    pub(super) genesis_l1_height: u64,

    /// The depth at which we accept blocks to be finalized.
    pub(super) finalization_depth: u64,

    /// The epoch that we've emitted as the final epoch.
    pub(super) declared_final_epoch: Option<EpochCommitment>,

    /// Internal states according to each block height.
    pub(crate) int_states: StateQueue<InternalState>,
}

impl ClientState {
    /// Creates the basic genesis client state from the genesis parameters.
    // TODO do we need this or should we load it at run time from the rollup params?
    pub fn from_genesis_params(params: &Params, gblkid: L2BlockId) -> Self {
        let rparams = params.rollup();
        let sync_state = SyncState::from_genesis_blkid(gblkid);
        let genesis_l1_height = rparams.genesis_l1_view.blk.height();
        Self {
            genesis_blockid: None,
            genesis_l1_height,
            finalization_depth: rparams.l1_reorg_safe_depth as u64,
            declared_final_epoch: None,
            int_states: StateQueue::new_at_index(genesis_l1_height),
        }
    }

    /// Returns a genesis block id, if already known.
    pub fn genesis_blkid(&self) -> Option<L2BlockId> {
        self.genesis_blockid
    }

    /// FIXME: remove usage of this
    pub fn has_genesis_occurred(&self) -> bool {
        self.genesis_blockid.is_some()
    }

    /// Overwrites the sync state.
    pub fn set_genesis_block(&mut self, genesis_blkid: L2BlockId) {
        self.genesis_blockid = Some(genesis_blkid);
    }

    pub fn most_recent_l1_block(&self) -> Option<&L1BlockId> {
        self.int_states.back().map(|is| is.blkid())
    }

    pub fn next_exp_l1_block(&self) -> u64 {
        self.int_states.next_idx()
    }

    pub fn genesis_l1_height(&self) -> u64 {
        self.genesis_l1_height
    }

    /// Gets the internal state for a height, if present.
    pub fn get_internal_state(&self, height: u64) -> Option<&InternalState> {
        self.int_states.get_absolute(height)
    }

    /// Gets the number of internal states tracked.
    pub fn internal_state_cnt(&self) -> usize {
        self.int_states.len()
    }

    /// Returns the deepest L1 block we have, if there is one.
    pub fn get_deepest_l1_block(&self) -> Option<L1BlockCommitment> {
        self.int_states
            .front_entry()
            .map(|(h, is)| L1BlockCommitment::new(h, is.blkid))
    }

    /// Returns the deepest L1 block we have, if there is one.
    pub fn get_tip_l1_block(&self) -> Option<L1BlockCommitment> {
        self.int_states
            .back_entry()
            .map(|(h, is)| L1BlockCommitment::new(h, is.blkid))
    }

    /// Gets the highest internal state we have.
    ///
    /// This isn't durable, as it's possible it might be rolled back in the
    /// future.
    pub fn get_last_internal_state(&self) -> Option<&InternalState> {
        self.int_states.back()
    }

    /// Gets the last checkpoint as of the last internal state.
    ///
    /// This isn't durable, as it's possible it might be rolled back in the
    /// future, although it becomes less likely the longer it's buried.
    pub fn get_last_checkpoint(&self) -> Option<&L1Checkpoint> {
        self.get_last_internal_state()
            .and_then(|st| st.last_checkpoint())
    }

    /// Gets the height that an L2 block was last verified at, if it was
    /// verified.
    // FIXME this is a weird function, what purpose does this serve?
    pub fn get_verified_l1_height(&self, slot: u64) -> Option<u64> {
        self.get_last_checkpoint().and_then(|ckpt| {
            if ckpt.batch_info.includes_l2_block(slot) {
                Some(ckpt.l1_reference.block_height())
            } else {
                None
            }
        })
    }

    /// Gets the last checkpoint as of some depth.  This depth is relative to
    /// the current L1 tip.  A depth of 0 would refer to the current L1 tip
    /// block.
    pub fn get_last_checkpoint_at_depth(&self, depth: u64) -> Option<&L1Checkpoint> {
        let cur_height = self.get_tip_l1_block()?.height();
        let target = cur_height - depth;
        self.get_internal_state(target)?.last_checkpoint()
    }

    /// Gets the apparent finalized checkpoint based on our current view of L1
    /// from the internal states.
    ///
    /// This uses the internal "finalization depth", checking relative to the
    /// current chain tip.
    pub fn get_apparent_finalized_checkpoint(&self) -> Option<&L1Checkpoint> {
        self.get_last_checkpoint_at_depth(self.finalization_depth)
    }

    /// Gets the `EpochCommitment` for the finalized epoch, if there is one.
    pub fn get_apparent_finalized_epoch(&self) -> Option<EpochCommitment> {
        self.get_apparent_finalized_checkpoint()
            .map(|ck| ck.batch_info.get_epoch_commitment())
    }

    /// Gets the L1 block we treat as buried, if there is one and we have it.
    pub fn get_buried_l1_block(&self) -> Option<L1BlockCommitment> {
        let tip_block = self.get_tip_l1_block()?;
        let buried_height = tip_block.height().saturating_sub(self.finalization_depth);
        let istate = self.get_internal_state(buried_height)?;
        Some(L1BlockCommitment::new(buried_height, *istate.blkid()))
    }

    /// Gets the final epoch that we've externally declared.
    pub fn get_declared_final_epoch(&self) -> Option<&EpochCommitment> {
        self.declared_final_epoch.as_ref()
    }
}

/// This is the internal state that's produced as the output of a block and
/// tracked internally.  When the L1 reorgs, we discard copies of this after the
/// reorg.
///
/// Eventually, when we do away with global bookkeeping around client state,
/// this will become the full client state that we determine on the fly based on
/// what L1 blocks are available and what we have persisted.
#[derive(
    Clone, Debug, Eq, PartialEq, Arbitrary, BorshSerialize, BorshDeserialize, Deserialize, Serialize,
)]
pub struct InternalState {
    /// Corresponding block ID.  This entry is stored keyed by height, so we
    /// always have that.
    blkid: L1BlockId,

    /// Last checkpoint as of this height.  Includes the height it was found at.
    ///
    /// At genesis, this is `None`.
    last_checkpoint: Option<L1Checkpoint>,
}

impl InternalState {
    pub fn new(blkid: L1BlockId, last_checkpoint: Option<L1Checkpoint>) -> Self {
        Self {
            blkid,
            last_checkpoint,
        }
    }

    /// Returns a ref to the L1 block ID that produced this state.
    pub fn blkid(&self) -> &L1BlockId {
        &self.blkid
    }

    /// Returns the last stored checkpoint, if there was one.
    pub fn last_checkpoint(&self) -> Option<&L1Checkpoint> {
        self.last_checkpoint.as_ref()
    }

    /// Returns the last known epoch as of this state.
    ///
    /// If there is no last epoch, returns a null epoch.
    pub fn get_last_epoch(&self) -> EpochCommitment {
        self.last_checkpoint
            .as_ref()
            .map(|ck| ck.batch_info.get_epoch_commitment())
            .unwrap_or_else(EpochCommitment::null)
    }

    /// Gets the next epoch we expect to be confirmed.
    pub fn get_next_expected_epoch_conf(&self) -> u64 {
        let last_epoch = self.get_last_epoch();
        if last_epoch.is_null() {
            0
        } else {
            last_epoch.epoch() + 1
        }
    }

    /// Returns the last witnessed L1 block from the last checkpointed state.
    pub fn last_witnessed_l1_block(&self) -> Option<&L1BlockCommitment> {
        self.last_checkpoint
            .as_ref()
            .map(|ck| ck.batch_info.final_l1_block())
    }
}

/// Represents a reference to a transaction in bitcoin. Redundantly puts block_height a well.
#[derive(
    Clone, Debug, Eq, PartialEq, Arbitrary, BorshDeserialize, BorshSerialize, Deserialize, Serialize,
)]
pub struct CheckpointL1Ref {
    pub l1_commitment: L1BlockCommitment,
    pub txid: Buf32,
    pub wtxid: Buf32,
}

impl CheckpointL1Ref {
    pub fn new(l1_commitment: L1BlockCommitment, txid: Buf32, wtxid: Buf32) -> Self {
        Self {
            l1_commitment,
            txid,
            wtxid,
        }
    }

    pub fn block_height(&self) -> u64 {
        self.l1_commitment.height()
    }

    pub fn block_id(&self) -> &L1BlockId {
        self.l1_commitment.blkid()
    }
}

#[derive(
    Clone, Debug, Eq, PartialEq, Arbitrary, BorshDeserialize, BorshSerialize, Deserialize, Serialize,
)]
pub struct L1Checkpoint {
    /// The inner checkpoint batch info.
    pub batch_info: BatchInfo,

    /// The inner checkpoint batch transition.
    pub batch_transition: BatchTransition,

    /// L1 reference for this checkpoint.
    pub l1_reference: CheckpointL1Ref,
}

impl L1Checkpoint {
    pub fn new(
        batch_info: BatchInfo,
        batch_transition: BatchTransition,
        l1_reference: CheckpointL1Ref,
    ) -> Self {
        Self {
            batch_info,
            batch_transition,
            l1_reference,
        }
    }
}

/// Wrapper around [`ClientState`] used for modifying it and producing sync
/// actions.
#[derive(Debug)]
pub struct ClientStateMut {
    state: ClientState,
    actions: Vec<SyncAction>,
}

impl ClientStateMut {
    pub fn new(state: ClientState) -> Self {
        Self {
            state,
            actions: Vec::new(),
        }
    }

    pub fn state(&self) -> &ClientState {
        &self.state
    }

    pub fn into_update(self) -> ClientUpdateOutput {
        ClientUpdateOutput::new(self.state, self.actions)
    }

    pub fn push_action(&mut self, a: SyncAction) {
        self.actions.push(a);
    }

    pub fn push_actions(&mut self, a: impl Iterator<Item = SyncAction>) {
        self.actions.extend(a);
    }

    // Semantical mutation fns.
    // TODO remove logs from this, break down into simpler logical units

    pub fn set_genesis_block(&mut self, genesis_blkid: L2BlockId) {
        self.state.set_genesis_block(genesis_blkid);
    }

    /// Rolls back blocks and stuff to a particular height.
    ///
    /// # Panics
    ///
    /// If the new height is below the buried height or it's somehow otherwise
    /// unable to perform the rollback.
    pub fn rollback_l1_blocks(&mut self, new_block: L1BlockCommitment) {
        let deepest_block = self
            .state
            .get_deepest_l1_block()
            .expect("clientstate: rolling back without blocks");

        // TODO: should this be removed ?
        let _cur_tip_block = self
            .state
            .get_tip_l1_block()
            .expect("clientstate: rolling back without blocks");

        if new_block.height() < deepest_block.height() {
            panic!("clientstate: tried to roll back past deepest block");
        }

        let remove_start_height = new_block.height() + 1;
        assert!(
            self.state.int_states.truncate_abs(remove_start_height),
            "clientstate: remove reorged blocks"
        );
    }

    /// Accepts a new L1 block that extends the chain directly.
    ///
    /// # Panics
    ///
    /// * If the blkids are inconsistent.
    /// * If the block already has a corresponding state.
    /// * If there isn't a preceding block.
    pub fn accept_l1_block_state(&mut self, l1block: &L1BlockCommitment, intstate: InternalState) {
        let h = l1block.height();
        let int_states = &mut self.state.int_states;

        if int_states.is_empty() {
            // Sanity checks.
            assert_eq!(
                l1block.blkid(),
                intstate.blkid(),
                "clientstate: inserting invalid block state"
            );

            assert_eq!(
                int_states.next_idx(),
                h,
                "clientstate: inserting out of order block state"
            );
        }

        let new_h = int_states.push_back(intstate);

        // Extra, probably redundant, sanity check.
        assert_eq!(
            new_h, h,
            "clientstate: inserted block state is for unexpected height"
        );
    }

    /// Discards old block states up to a certain height which becomes the new oldest.
    ///
    /// # Panics
    ///
    /// * If trying to discard the newest.
    /// * If there are no states to discard, for any reason.
    pub fn discard_old_l1_states(&mut self, new_oldest: u64) {
        let int_states = &mut self.state.int_states;

        let oldest = int_states
            .front_idx()
            .expect("clientstate: missing expected block state");

        let newest = int_states
            .back_idx()
            .expect("clientstate: missing expected block state");

        if new_oldest <= oldest {
            panic!("clientstate: discard earlier than oldest state ({new_oldest})");
        }

        if new_oldest >= newest {
            panic!("clientstate: discard newer than newest state ({new_oldest})");
        }

        // Actually do the operation.
        int_states.drop_abs(new_oldest);

        // Sanity checks.
        assert_eq!(
            int_states.front_idx(),
            Some(new_oldest),
            "chainstate: new oldest is unexpected"
        );
    }

    /// Sets the declared final epoch.
    pub fn set_decl_final_epoch(&mut self, epoch: EpochCommitment) {
        self.state.declared_final_epoch = Some(epoch);
    }
}
