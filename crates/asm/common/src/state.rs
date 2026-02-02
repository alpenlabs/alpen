use borsh::{BorshDeserialize, BorshSerialize};
use rkyv::{
    Archived, Place, Resolver,
    rancor::Fallible,
    with::{ArchiveWith, DeserializeWith, SerializeWith},
};
use serde::{Deserialize, Serialize};
use strata_asm_types::HeaderVerificationState;

use crate::{AsmError, AsmHistoryAccumulatorState, Mismatched, Subprotocol, SubprotocolId};

/// Serializer for any type that implements [`Serialize`] as JSON bytes for rkyv.
struct SerdeJsonBytes;

impl<T> ArchiveWith<T> for SerdeJsonBytes
where
    T: Serialize,
{
    type Archived = Archived<Vec<u8>>;
    type Resolver = Resolver<Vec<u8>>;

    fn resolve_with(field: &T, resolver: Self::Resolver, out: Place<Self::Archived>) {
        let bytes = serde_json::to_vec(field).expect("serde_json should serialize ASM accumulator");
        rkyv::Archive::resolve(&bytes, resolver, out);
    }
}

impl<T, S> SerializeWith<T, S> for SerdeJsonBytes
where
    T: Serialize,
    S: Fallible + ?Sized,
    Vec<u8>: rkyv::Serialize<S>,
{
    fn serialize_with(field: &T, serializer: &mut S) -> Result<Self::Resolver, S::Error> {
        let bytes = serde_json::to_vec(field).expect("serde_json should serialize ASM accumulator");
        rkyv::Serialize::serialize(&bytes, serializer)
    }
}

impl<T, D> DeserializeWith<Archived<Vec<u8>>, T, D> for SerdeJsonBytes
where
    for<'de> T: Deserialize<'de>,
    D: Fallible + ?Sized,
    Archived<Vec<u8>>: rkyv::Deserialize<Vec<u8>, D>,
{
    fn deserialize_with(field: &Archived<Vec<u8>>, deserializer: &mut D) -> Result<T, D::Error> {
        let bytes = rkyv::Deserialize::deserialize(field, deserializer)?;
        Ok(serde_json::from_slice(&bytes).expect("serde_json should deserialize ASM accumulator"))
    }
}

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
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
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
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
pub struct ChainViewState {
    /// All data needed to validate a Bitcoin block header, including past‐n timestamps,
    /// accumulated work, and difficulty adjustments.
    pub pow_state: HeaderVerificationState,

    /// History accumulator tracking processed L1 blocks.
    ///
    /// Each leaf represents the root hash of an [`AsmManifest`](crate::AsmManifest) for the
    /// corresponding block, enabling efficient historical proofs of ASM state transitions.
    #[rkyv(with = SerdeJsonBytes)]
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
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
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
