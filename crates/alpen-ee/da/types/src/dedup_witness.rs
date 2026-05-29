//! Supplementary "dedup" witness: data the guest needs to verify a published DA
//! blob whose references aren't all carried inline.

use rkyv::{Archive, Deserialize, Serialize};
use rkyv_impl::archive_impl;

/// Extra data the guest needs to verify a published DA blob whose references
/// aren't carried inline.
///
/// DA dedup lets a batch omit data an earlier batch already published (today:
/// deployed bytecodes). A later account diff can still reference that data's
/// hash, so the host resupplies it here and the guest re-hashes it to confirm
/// the match. This confirms the bytes match the referenced hash — *not* that
/// they were published on L1 in an earlier batch.
///
/// TODO(STR-1907): prove prior publication via a membership proof against an
/// authenticated published-data set. Future dedup kinds (account/storage
/// serials) add their own fields here, each carrying such a proof.
#[derive(Clone, Debug, Default, Archive, Deserialize, Serialize)]
#[rkyv(derive(Debug))]
pub struct DedupWitness {
    /// Bytecodes omitted from the current DA blob because they were already
    /// published in a prior batch, supplied so the guest can verify account
    /// diffs that still reference them.
    deduped_bytecode_preimages: Vec<BytecodePreimage>,
}

impl DedupWitness {
    pub fn new(deduped_bytecode_preimages: Vec<BytecodePreimage>) -> Self {
        Self {
            deduped_bytecode_preimages,
        }
    }

    pub fn empty() -> Self {
        Self::default()
    }

    pub fn deduped_bytecode_preimages(&self) -> &[BytecodePreimage] {
        &self.deduped_bytecode_preimages
    }
}

impl ArchivedDedupWitness {
    pub fn deduped_bytecode_preimages(&self) -> &[ArchivedBytecodePreimage] {
        &self.deduped_bytecode_preimages
    }
}

/// Preimage for a bytecode the blob's state diff references but omits (DA dedup).
///
/// The public DA blob may omit a bytecode when its hash was already published in
/// an earlier batch, but a later account diff can still set that same code hash.
/// The matching code hash is `keccak256(bytecode)`; the guest recomputes it
/// rather than trusting a stored value, so no hash is carried here.
///
/// NOTE: this proves bytecode identity, not prior L1 publication.
/// TODO(STR-1907): replace with a membership proof against an authenticated
/// published-bytecode set.
#[derive(Clone, Debug, Eq, PartialEq, Archive, Deserialize, Serialize)]
#[rkyv(derive(Debug))]
pub struct BytecodePreimage {
    bytecode: Vec<u8>,
}

impl BytecodePreimage {
    pub fn new(bytecode: Vec<u8>) -> Self {
        Self { bytecode }
    }
}

#[archive_impl]
impl BytecodePreimage {
    pub fn bytecode(&self) -> &[u8] {
        &self.bytecode
    }
}

#[cfg(test)]
mod tests {
    use rkyv::rancor::Error as RkyvError;

    use super::*;

    #[test]
    fn dedup_witness_with_bytecode_preimage_roundtrips_through_rkyv() {
        let witness = DedupWitness::new(vec![BytecodePreimage::new(vec![0x60, 0x80])]);

        let bytes = rkyv::to_bytes::<RkyvError>(&witness).unwrap();
        let archived = rkyv::access::<ArchivedDedupWitness, RkyvError>(&bytes).unwrap();

        let preimages = archived.deduped_bytecode_preimages();
        assert_eq!(preimages.len(), 1);
        assert_eq!(preimages[0].bytecode(), &[0x60, 0x80]);
    }
}
