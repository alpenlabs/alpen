//! Operations that a state transition emits to update the new state and control
//! the client's high level state.

use arbitrary::Arbitrary;
use serde::{Deserialize, Serialize};

use crate::client_state::ClientState;

/// Output of a consensus state transition. Right now it consists of full [`ClientState`] and
/// sync actions.
#[derive(Clone, Debug, Eq, PartialEq, Arbitrary, Deserialize, Serialize)]
pub struct ClientUpdateOutput {
    state: ClientState,
}

impl ClientUpdateOutput {
    pub fn new_state(state: ClientState) -> Self {
        Self { state }
    }

    pub fn state(&self) -> &ClientState {
        &self.state
    }

    pub fn into_state(self) -> ClientState {
        self.state
    }

    pub fn into_parts(self) -> (ClientState, Vec<()>) {
        (self.state, Vec::new())
    }
}
