use std::io::Error as IoError;

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use ssz::{Decode, Encode};
use ssz_derive::{Decode as DeriveDecode, Encode as DeriveEncode};
use ssz_types::VariableList;
use strata_btc_verification::HeaderVerificationState;
use tree_hash::{PackedEncoding, Sha256Hasher, TreeHash, TreeHashType};
use tree_hash_derive::TreeHash;

use crate::{AsmError, AsmHistoryAccumulatorState, Mismatched, Subprotocol, SubprotocolId};

/// Anchor state for the Anchor State Machine (ASM), the core of the Strata protocol.
///
/// The ASM anchors the orchestration layer to L1, akin to a host smart contract
/// in an EVM environment. It defines a pure state transition function (STF)
/// over L1 blocks: given a prior ASM state and a new L1 block, it computes the
/// next ASM state off-chain. Conceptually, this is like a stateful smart contract
/// receiving protocol transactions at L1 and updating its storage. A zk-SNARK proof
/// attests that the transition from the previous ASM state to the new state
/// was performed correctly on the given L1 block.
#[derive(
    Clone,
    Debug,
    PartialEq,
    BorshSerialize,
    BorshDeserialize,
    Serialize,
    Deserialize,
    DeriveEncode,
    DeriveDecode,
)]
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
#[derive(
    Clone,
    Debug,
    PartialEq,
    BorshSerialize,
    BorshDeserialize,
    Serialize,
    Deserialize,
    DeriveEncode,
    DeriveDecode,
    TreeHash,
)]
pub struct ChainViewState {
    /// All data needed to validate a Bitcoin block header, including past‐n timestamps,
    /// accumulated work, and difficulty adjustments.
    pub pow_state: HeaderVerificationState,

    /// History accumulator tracking processed L1 blocks.
    ///
    /// Each leaf represents the root hash of an [`AsmManifest`](crate::AsmManifest) for the
    /// corresponding block, enabling efficient historical proofs of ASM state transitions.
    pub history_accumulator: AsmHistoryAccumulatorState,
}

impl ChainViewState {
    /// Destructures the chain view into its constituent parts.
    pub fn into_parts(self) -> (HeaderVerificationState, AsmHistoryAccumulatorState) {
        (self.pow_state, self.history_accumulator)
    }
}

/// Holds the off‐chain serialized state for a single subprotocol section within the ASM.
///
/// Each `SectionState` pairs the subprotocol’s unique ID with its current serialized state,
/// allowing the ASM to apply the appropriate state transition logic for that subprotocol.
#[derive(
    Clone,
    Debug,
    PartialEq,
    BorshSerialize,
    BorshDeserialize,
    Serialize,
    Deserialize,
    DeriveEncode,
    DeriveDecode,
)]
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
        Self::new(S::ID, state.as_ssz_bytes())
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

        <S::State as Decode>::from_ssz_bytes(&self.data)
            .map_err(|e| AsmError::Deserialization(self.id, IoError::other(e.to_string())))
    }
}

/// The maximum number of bytes for a section state.
const MAX_SECTION_STATE_BYTES: usize = 1 << 20;

/// The maximum number of sections.
const MAX_SECTIONS: usize = 256;

/// The [`TreeHash`] representation of the [`SectionState`].
#[derive(TreeHash)]
struct SectionStateTreeHash {
    /// The subprotocol ID.
    id: SubprotocolId,

    /// The serialized data.
    data: VariableList<u8, MAX_SECTION_STATE_BYTES>,
}

/// The [`TreeHash`] representation of the [`AnchorState`].
#[derive(TreeHash)]
struct AnchorStateTreeHash {
    /// The chain view.
    chain_view: ChainViewState,

    /// The sections.
    sections: VariableList<SectionState, MAX_SECTIONS>,
}

impl TreeHash for SectionState {
    fn tree_hash_type() -> TreeHashType {
        <SectionStateTreeHash as TreeHash>::tree_hash_type()
    }

    fn tree_hash_packed_encoding(&self) -> PackedEncoding {
        <SectionStateTreeHash as TreeHash>::tree_hash_packed_encoding(&SectionStateTreeHash {
            id: self.id,
            data: VariableList::from(self.data.clone()),
        })
    }

    fn tree_hash_packing_factor() -> usize {
        <SectionStateTreeHash as TreeHash>::tree_hash_packing_factor()
    }

    fn tree_hash_root(&self) -> <Sha256Hasher as tree_hash::TreeHashDigest>::Output {
        <SectionStateTreeHash as TreeHash>::tree_hash_root(&SectionStateTreeHash {
            id: self.id,
            data: VariableList::from(self.data.clone()),
        })
    }
}

impl TreeHash for AnchorState {
    fn tree_hash_type() -> TreeHashType {
        <AnchorStateTreeHash as TreeHash>::tree_hash_type()
    }

    fn tree_hash_packed_encoding(&self) -> PackedEncoding {
        <AnchorStateTreeHash as TreeHash>::tree_hash_packed_encoding(&AnchorStateTreeHash {
            chain_view: self.chain_view.clone(),
            sections: VariableList::from(self.sections.clone()),
        })
    }

    fn tree_hash_packing_factor() -> usize {
        <AnchorStateTreeHash as TreeHash>::tree_hash_packing_factor()
    }

    fn tree_hash_root(&self) -> <Sha256Hasher as tree_hash::TreeHashDigest>::Output {
        <AnchorStateTreeHash as TreeHash>::tree_hash_root(&AnchorStateTreeHash {
            chain_view: self.chain_view.clone(),
            sections: VariableList::from(self.sections.clone()),
        })
    }
}

#[cfg(test)]
mod tests {
    use ssz::{Decode, Encode};
    use tree_hash::{Sha256Hasher, TreeHash};

    use super::*;
    use crate::AsmHistoryAccumulatorState;

    fn sample_anchor_state() -> AnchorState {
        AnchorState {
            chain_view: ChainViewState {
                pow_state: HeaderVerificationState::default(),
                history_accumulator: AsmHistoryAccumulatorState::new(0),
            },
            sections: vec![SectionState::new(7, vec![1, 2, 3, 4])],
        }
    }

    #[test]
    fn test_anchor_state_ssz_roundtrip() {
        let state = sample_anchor_state();
        let bytes = state.as_ssz_bytes();
        let decoded = AnchorState::from_ssz_bytes(&bytes).unwrap();

        assert_eq!(state, decoded);
    }

    #[test]
    fn test_anchor_state_tree_hash_deterministic() {
        let state = sample_anchor_state();
        let hash1 = <AnchorState as TreeHash<Sha256Hasher>>::tree_hash_root(&state);
        let hash2 = <AnchorState as TreeHash<Sha256Hasher>>::tree_hash_root(&state);

        assert_eq!(hash1, hash2);
    }
}
