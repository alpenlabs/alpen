//! MMR (Merkle Mountain Range) identifier types.

use serde::{Deserialize, Serialize};

use crate::AccountId;

/// Identifier for a specific MMR instance in unified storage
///
/// Each variant represents a different MMR type, with optional scoping
/// within that type (e.g., per-account MMRs).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MmrId {
    /// ASM manifest MMR (singleton, no account scope)
    Asm,
    /// Snark message MMR (per-account scope)
    SnarkMsg(AccountId),
}

impl MmrId {
    /// Serialize MmrId to bytes for use as database key
    ///
    /// Uses bincode with big-endian encoding for compatibility with existing data.
    pub fn to_bytes(&self) -> Vec<u8> {
        use bincode::Options;
        let options = bincode::options().with_fixint_encoding().with_big_endian();
        options
            .serialize(self)
            .expect("MmrId serialization should not fail")
    }
}
