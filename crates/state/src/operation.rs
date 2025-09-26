//! Operations that a state transition emits to update the new state and control
//! the client's high level state.

use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_checkpoint_types::Checkpoint;
use strata_primitives::epoch::EpochCommitment;

use crate::client_state::{CheckpointL1Ref, ClientState};

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
#[expect(clippy::large_enum_variant, reason = "I don't want to box it")]
#[derive(
    Clone, Debug, Eq, PartialEq, Arbitrary, BorshDeserialize, BorshSerialize, Deserialize, Serialize,
)]
pub enum SyncAction {
    /// Finalizes an epoch, indicating that we won't revert it.
    ///
    /// This also implicitly finalizes all blocks preceding the epoch terminal.
    FinalizeEpoch(EpochCommitment),

    /// Checkpoint is included in L1 at given L1 reference.
    UpdateCheckpointInclusion {
        checkpoint: Checkpoint,
        l1_reference: CheckpointL1Ref,
    },
}
