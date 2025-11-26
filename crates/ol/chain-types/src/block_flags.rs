//! Flags field for block header.

use ssz_derive::{Decode, Encode};
use strata_identifiers::impl_ssz_transparent_wrapper;

type RawBlockFlags = u16;

const IS_TERMINAL: RawBlockFlags = 0x0001;

/// Flags in the block header that we use for various signalling purposes.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Encode, Decode)]
pub struct BlockFlags(RawBlockFlags);

impl_ssz_transparent_wrapper!(BlockFlags, u16, 2);

impl From<u16> for BlockFlags {
    fn from(value: u16) -> Self {
        Self(value)
    }
}

impl From<BlockFlags> for u16 {
    fn from(value: BlockFlags) -> Self {
        value.0
    }
}

impl BlockFlags {
    /// Constructs a zero flag.
    pub fn zero() -> Self {
        Self(0)
    }

    /// Assigns the `IS_TERMINAL` flag to some value.
    pub fn set_is_terminal(&mut self, b: bool) {
        if b {
            self.0 |= IS_TERMINAL;
        } else {
            self.0 &= !IS_TERMINAL;
        }
    }

    /// Checks if the `IS_TERMINAL` flag is set.
    pub fn is_terminal(&self) -> bool {
        self.0 & IS_TERMINAL != 0
    }
}
