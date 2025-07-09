use borsh::{BorshDeserialize, BorshSerialize};
use strata_l1_txfmt::SubprotocolId;

use crate::{AsmError, Mismatched, Subprotocol};

/// A single subprotocol’s auxiliary‐input payload, stored as raw Borsh‐serialized blobs.
///
/// `AuxPayload` contains everything a given subprotocol asked for during its
/// `pre_process_txs` phase, in opaque byte form. The `id` field tells you
/// which `Subprotocol::ID` this belongs to, and `data` is a list of raw
/// Borsh‐serialized AuxInput blobs—one blob per instance of that protocol’s
/// `AuxInput`.
///
/// Each entry in `data` must deserialize into an instance of
/// `<P as Subprotocol>::AuxInput`, so that
/// `payload.data` → `Vec<P::AuxInput>`.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct AuxPayload {
    /// Which subprotocol this payload belongs to.
    pub id: SubprotocolId,

    /// A list of Borsh‐serialized AuxInput blobs.
    ///
    /// Each `Vec<u8>` here must deserialize into one
    /// `<P as Subprotocol>::AuxInput`, so that the entire
    /// `data` vector becomes a `Vec<P::AuxInput>`.
    pub data: Vec<Vec<u8>>,
}

impl AuxPayload {
    pub fn new(id: SubprotocolId, data: Vec<Vec<u8>>) -> Self {
        Self { id, data }
    }

    pub fn try_to_aux_inputs<S: Subprotocol>(&self) -> Result<Vec<S::AuxInput>, AsmError> {
        if S::ID != self.id {
            return Err(Mismatched {
                expected: S::ID,
                actual: self.id,
            }
            .into());
        }
        self.data
            .iter()
            .map(|raw| {
                <S::AuxInput as BorshDeserialize>::try_from_slice(raw)
                    .map_err(|e| AsmError::Deserialization(self.id, e))
            })
            .collect()
    }
}

/// A bundle of auxiliary‐input payloads for a specific L1 block.
///
/// `AuxBundle` collects all of the `AuxPayload`s produced by each subprotocol
/// during their `pre_process_txs` phase for one particular L1 block. You can
/// use it to look up, decode, and feed each protocol’s inputs into
/// `process_txs`.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct AuxBundle {
    /// All auxiliary‐input payloads collected for this L1 block.
    pub entries: Vec<AuxPayload>,
}

impl AuxBundle {
    /// Gets a section by protocol ID by doing a linear scan.
    pub fn find_payload(&self, id: SubprotocolId) -> Option<&AuxPayload> {
        self.entries.iter().find(|s| s.id == id)
    }
}
