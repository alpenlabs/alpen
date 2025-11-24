//! Flags field for block header.

use strata_codec_derive::Codec;

type RawBlockFlags = u16;

const IS_TERMINAL: RawBlockFlags = 0x0001;

/// Flags in the block header that we use for various signalling purposes.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Codec)]
pub struct BlockFlags(RawBlockFlags);

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
