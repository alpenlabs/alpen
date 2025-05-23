use borsh::{BorshDeserialize, BorshSerialize};
use strata_primitives::l1::HeaderVerificationState;

use crate::{AsmError, Mismatched, Subprotocol, SubprotocolId};

/// Anchor state for the Anchor State Machine (ASM), the core of the Strata protocol.
///
/// The ASM anchors the orchestration layer to L1, akin to a host smart contract
/// in an EVM environment. It defines a pure state transition function (STF)
/// over L1 blocks: given a prior ASM state and a new L1 block, it computes the
/// next ASM state off-chain. Conceptually, this is like a stateful smart contract
/// receiving protocol transactions at L1 and updating its storage. A zk-SNARK proof
/// attests that the transition from the previous ASM state to the new state
/// was performed correctly on the given L1 block.
#[derive(Clone, Debug)]
pub struct AnchorState {
    /// The current view of the L1 chain required for state transitions.
    pub chain_view: ChainViewState,

    /// States for each subprotocol section, sorted by Subprotocol Version/ID.
    pub sections: Vec<SectionState>,
}

impl AnchorState {
    /// Gets a section by protocol ID by doing a linear scan.
    pub fn find_section(&self, id: SubprotocolId) -> Option<&SectionState> {
        self.sections.iter().find(|s| s.id == id)
    }
}

/// Represents the on‐chain view required by the Anchor State Machine (ASM) to process
/// state transitions for each new L1 block.
#[derive(Clone, Debug)]
pub struct ChainViewState {
    /// All data needed to validate a Bitcoin block header, including past‐n timestamps,
    /// accumulated work, and difficulty adjustments.
    pub pow_state: HeaderVerificationState,
    // TODO header MMR
}

/// Holds the off‐chain serialized state for a single subprotocol section within the ASM.
///
/// Each `SectionState` pairs the subprotocol’s unique ID with its current serialized state,
/// allowing the ASM to apply the appropriate state transition logic for that subprotocol.
#[derive(Clone, Debug)]
pub struct SectionState {
    /// Identifier of the subprotocol
    pub id: SubprotocolId,

    /// The serialized subprotocol state.
    ///
    /// This is normally fairly small, but we are setting a comfortable max limit.
    pub data: Vec<u8>,
}

impl SectionState {
    /// Constructs a new instance.
    pub fn new(id: SubprotocolId, data: Vec<u8>) -> Self {
        Self { id, data }
    }

    /// Constructs an instance by serializing a subprotocol state.
    pub fn from_state<S: Subprotocol>(state: &S::State) -> Self {
        let mut buf = Vec::new();
        <S::State as BorshSerialize>::serialize(state, &mut buf).expect("asm: serialize");
        Self::new(S::ID, buf)
    }

    /// Tries to deserialize the section data as a particular subprotocol's state.
    pub fn try_to_state<S: Subprotocol>(&self) -> Result<S::State, AsmError> {
        if S::ID != self.id {
            return Err(Mismatched {
                expected: S::ID,
                actual: self.id,
            }
            .into());
        }

        <S::State as BorshDeserialize>::try_from_slice(&self.data)
            .map_err(|e| AsmError::Deserialization(self.id, e))
    }
}
