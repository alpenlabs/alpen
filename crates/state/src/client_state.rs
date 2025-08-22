//! Consensus types that track node behavior as we receive messages from the L1
//! chain and the p2p network. These will be expanded further as we actually
//! implement the consensus logic.
// TODO move this to another crate that contains our sync logic

use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_primitives::{
    batch::BatchTransition, buf::Buf32, epoch::EpochCommitment, l1::L1BlockCommitment,
};

use crate::{
    batch::BatchInfo,
    l1::L1BlockId,
    operation::{ClientUpdateOutput, SyncAction},
};

/// High level client's state of the network. This is local to the client, not
/// coordinated as part of the L2 chain.
///
/// This is updated when we see a consensus-relevant message.  This is L2 blocks
/// but also L1 blocks being published with relevant things in them, and
/// various other events.
#[derive(
    Clone, Debug, Eq, PartialEq, Arbitrary, BorshSerialize, BorshDeserialize, Deserialize, Serialize,
)]
pub struct ClientState {
    // Last *finalized*
    pub(crate) last_finalized_checkpoint: Option<L1Checkpoint>,

    /// Height
    /// TODO(QQ): Currently weird, as it's already keyed by [`L1BlockCommitment`]
    pub(crate) height: u64,
}

impl ClientState {
    /// Creates the basic genesis client state from the genesis parameters.
    // TODO do we need this or should we load it at run time from the rollup params?
    pub fn new() -> Self {
        Self {
            last_finalized_checkpoint: None,
            height: 0,
        }
    }

    /// Returns if genesis has occurred.
    pub fn has_genesis_occurred(&self) -> bool {
        self.height > 0
    }

    pub fn next_exp_l1_block(&self) -> u64 {
        self.height + 1
    }

    pub fn height(&self) -> u64 {
        self.height
    }

    /// Gets the last checkpoint as of the last internal state.
    ///
    /// This isn't durable, as it's possible it might be rolled back in the
    /// future, although it becomes less likely the longer it's buried.
    pub fn get_last_checkpoint(&self) -> Option<L1Checkpoint> {
        self.last_finalized_checkpoint.clone()
    }

    /// Gets the final epoch that we've externally declared.
    pub fn get_declared_final_epoch(&self) -> Option<EpochCommitment> {
        self.get_last_checkpoint()
            .and_then(|ckpt| Some(ckpt.batch_info.get_epoch_commitment()))
    }

    /// Returns the last known epoch as of this state.
    ///
    /// If there is no last epoch, returns a null epoch.
    pub fn get_last_epoch(&self) -> EpochCommitment {
        self.last_finalized_checkpoint
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

    // TODO(QQ): it's very ugly, refactor?
    pub fn take_client_state(&mut self, client_state: ClientState) {
        self.state = client_state;
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

    /// Accepts a new L1 block that extends the chain directly.
    ///
    /// # Panics
    ///
    /// * If the blkids are inconsistent.
    /// * If the block already has a corresponding state.
    /// * If there isn't a preceding block.
    pub fn accept_l1_block_state(
        &mut self,
        l1block: &L1BlockCommitment,
        last_finalized_checkpoint: Option<L1Checkpoint>,
    ) {
        self.state.height = l1block.height();
        self.state.last_finalized_checkpoint = last_finalized_checkpoint;
    }
}
