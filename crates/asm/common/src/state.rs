use rkyv::{
    Archived, Place, Resolver,
    api::high::{HighDeserializer, HighSerializer},
    rancor::{Error as RkyvError, Fallible},
    ser::allocator::ArenaHandle,
    util::AlignedVec,
    with::{ArchiveWith, DeserializeWith, SerializeWith},
};
use serde::{Deserialize, Serialize};
use strata_asm_types::HeaderVerificationState;
use strata_codec::{decode_buf_exact, encode_to_vec};

use crate::{AsmError, AsmHistoryAccumulatorState, Mismatched, Subprotocol, SubprotocolId};

/// Serializer for any type that implements [`Serialize`] as codec bytes for rkyv.
pub type StateSerializer<'a> = HighSerializer<AlignedVec, ArenaHandle<'a>, RkyvError>;

/// Serializer for any type that implements [`Deserialize`] as codec bytes for rkyv.
pub type StateDeserializer = HighDeserializer<RkyvError>;

/// Trait for types that can be serialized and deserialized as state for rkyv.
pub trait RkyvState: rkyv::Archive + for<'a> rkyv::Serialize<StateSerializer<'a>> + Sized {}

impl<T> RkyvState for T where T: rkyv::Archive + for<'a> rkyv::Serialize<StateSerializer<'a>> + Sized
{}

/// Serializer for ASM history accumulator as codec bytes for rkyv.
struct AsmHistoryAccumulatorAsBytes;

impl ArchiveWith<AsmHistoryAccumulatorState> for AsmHistoryAccumulatorAsBytes {
    type Archived = Archived<Vec<u8>>;
    type Resolver = Resolver<Vec<u8>>;

    fn resolve_with(
        field: &AsmHistoryAccumulatorState,
        resolver: Self::Resolver,
        out: Place<Self::Archived>,
    ) {
        let bytes = encode_to_vec(field).expect("codec should serialize ASM history accumulator");
        rkyv::Archive::resolve(&bytes, resolver, out);
    }
}

impl<S> SerializeWith<AsmHistoryAccumulatorState, S> for AsmHistoryAccumulatorAsBytes
where
    S: Fallible + ?Sized,
    Vec<u8>: rkyv::Serialize<S>,
{
    fn serialize_with(
        field: &AsmHistoryAccumulatorState,
        serializer: &mut S,
    ) -> Result<Self::Resolver, S::Error> {
        let bytes = encode_to_vec(field).expect("codec should serialize ASM history accumulator");
        rkyv::Serialize::serialize(&bytes, serializer)
    }
}

impl<D> DeserializeWith<Archived<Vec<u8>>, AsmHistoryAccumulatorState, D>
    for AsmHistoryAccumulatorAsBytes
where
    D: Fallible + ?Sized,
    Archived<Vec<u8>>: rkyv::Deserialize<Vec<u8>, D>,
{
    fn deserialize_with(
        field: &Archived<Vec<u8>>,
        deserializer: &mut D,
    ) -> Result<AsmHistoryAccumulatorState, D::Error> {
        let bytes = rkyv::Deserialize::deserialize(field, deserializer)?;
        Ok(decode_buf_exact(&bytes).expect("codec should deserialize ASM history accumulator"))
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
    #[rkyv(with = AsmHistoryAccumulatorAsBytes)]
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
/// Each [`SectionState`] pairs the subprotocol’s unique ID with its current serialized state,
/// allowing the ASM to apply the appropriate state transition logic for that subprotocol.
#[derive(
    Clone,
    Debug,
    PartialEq,
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
    pub fn from_state<S: Subprotocol>(state: &S::State) -> Self
    where
        S::State: rkyv::Archive + for<'a> rkyv::Serialize<StateSerializer<'a>>,
    {
        let bytes = rkyv::to_bytes::<RkyvError>(state)
            .map_err(|err| AsmError::Serialization(S::ID, err))
            .expect("asm: serialize");
        Self::new(S::ID, bytes.as_ref().to_vec())
    }

    /// Tries to deserialize the section data as a particular subprotocol's state.
    pub fn try_to_state<S: Subprotocol>(&self) -> Result<S::State, AsmError>
    where
        S::State: rkyv::Archive,
        rkyv::Archived<S::State>: rkyv::Deserialize<S::State, StateDeserializer>,
    {
        if S::ID != self.id {
            return Err(Mismatched {
                expected: S::ID,
                actual: self.id,
            }
            .into());
        }

        // SAFETY: The serialized section bytes are produced internally by rkyv and are trusted.
        unsafe { rkyv::from_bytes_unchecked::<S::State, RkyvError>(&self.data) }
            .map_err(|e| AsmError::Deserialization(self.id, e))
    }
}
