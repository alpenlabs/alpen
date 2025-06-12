//! Operations that a state transition emits to update the new state and control
//! the client's high level state.

use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_primitives::{batch::Checkpoint, epoch::EpochCommitment};

use crate::{
    client_state::{CheckpointL1Ref, ClientState},
    l1::L1BlockId,
};

/// Output of a consensus state transition. Right now it consists of full [`ClientState`] and
/// sync actions.
#[derive(
    Clone, Debug, Eq, PartialEq, Arbitrary, BorshDeserialize, BorshSerialize, Deserialize, Serialize,
)]
pub struct ClientUpdateOutput {
    state: ClientState,
    actions: Vec<SyncAction>,
}

impl ClientUpdateOutput {
    pub fn new(state: ClientState, actions: Vec<SyncAction>) -> Self {
        Self { state, actions }
    }

    pub fn new_state(state: ClientState) -> Self {
        Self::new(state, Vec::new())
    }

    pub fn state(&self) -> &ClientState {
        &self.state
    }

    #[allow(clippy::missing_const_for_fn)]
    pub fn actions(&self) -> &[SyncAction] {
        &self.actions
    }

    pub fn into_state(self) -> ClientState {
        self.state
    }

    pub fn into_parts(self) -> (ClientState, Vec<SyncAction>) {
        (self.state, self.actions)
    }
}

/// Actions the client state machine directs the node to take to update its own
/// database bookkeeping.
#[allow(clippy::large_enum_variant)]
#[derive(
    Clone, Debug, Eq, PartialEq, Arbitrary, BorshDeserialize, BorshSerialize, Deserialize, Serialize,
)]
pub enum SyncAction {
    /// Finalizes an epoch, indicating that we won't revert it.
    ///
    /// This also implicitly finalizes all blocks preceding the epoch terminal.
    FinalizeEpoch(EpochCommitment),

    /// Indicates to the worker that it's safe to perform the L2 genesis
    /// operations and start the chain sync work, using a particular L1 block
    /// as the genesis lock-in block.
    L2Genesis(L1BlockId),

    /// Checkpoint is included in L1 at given L1 reference.
    UpdateCheckpointInclusion {
        checkpoint: Checkpoint,
        l1_reference: CheckpointL1Ref,
    },
}
