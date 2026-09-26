//! The fixed root of the OL state.

use strata_identifiers::Buf32;
use tree_hash::{Sha256Hasher, TreeHash};

use crate::spec_id::{OLSpecId, UnknownOLSpecId};
use crate::ssz_generated::ssz::root::OLRootState;

impl OLRootState {
    /// Creates a root from its raw fields.
    pub fn new(cur_spec_version: u32, staged_spec_version: u32, chainstate_root: Buf32) -> Self {
        let chainstate_root: [u8; 32] = chainstate_root.into();
        Self {
            cur_spec_version,
            staged_spec_version,
            chainstate_root: chainstate_root.into(),
        }
    }

    /// Returns the raw spec version the chainstate was produced under.
    pub fn cur_spec_version(&self) -> u32 {
        self.cur_spec_version
    }

    /// Returns the raw spec version the next epoch runs under.
    pub fn staged_spec_version(&self) -> u32 {
        self.staged_spec_version
    }

    /// Returns the committed chainstate root.
    pub fn chainstate_root(&self) -> Buf32 {
        let bytes: &[u8] = self.chainstate_root.as_ref();
        let bytes: [u8; 32] = bytes.try_into().expect("FixedBytes<32> is always 32 bytes");
        Buf32::from(bytes)
    }

    /// Converts [`Self::cur_spec_version`] to the spec it names.
    pub fn cur_spec(&self) -> Result<OLSpecId, UnknownOLSpecId> {
        OLSpecId::try_from(self.cur_spec_version)
    }

    /// Converts [`Self::staged_spec_version`] to the spec it names.
    pub fn staged_spec(&self) -> Result<OLSpecId, UnknownOLSpecId> {
        OLSpecId::try_from(self.staged_spec_version)
    }

    /// Computes the protocol state root, the SSZ hash tree root of this
    /// container.
    pub fn compute_state_root(&self) -> Buf32 {
        TreeHash::tree_hash_root::<Sha256Hasher>(self).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    /// Pins the protocol state root formula. A change to the field order,
    /// widths, or hashing changes every state root and needs a network reset.
    #[test]
    fn test_state_root_vectors() {
        let genesis = OLRootState::new(1, 1, Buf32::zero());
        assert_eq!(
            hex(genesis.compute_state_root().as_ref()),
            "b952e6507a336e798c422a22b66456a6439f8064c17715d8dfff9e99b0293a83"
        );

        let staged = OLRootState::new(1, 2, Buf32::from([0x11; 32]));
        assert_eq!(
            hex(staged.compute_state_root().as_ref()),
            "1613611df770dc64ce28e27571aebe8cbe6b72253ae5f9b41de3bc8fe7d1680a"
        );
    }
}
