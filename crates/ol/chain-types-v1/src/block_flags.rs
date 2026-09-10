//! Flags field for block header.

use ssz::DecodeError;
use strata_identifiers::{SszDelegate, impl_ssz_via_delegate};

type RawBlockFlags = u16;

const IS_TERMINAL: RawBlockFlags = 0x0001;
const KNOWN_FLAGS: RawBlockFlags = IS_TERMINAL;

/// Flags in the block header that we use for various signalling purposes.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct BlockFlagsV1(RawBlockFlags);

/// Reports that a block flags value contains bits unknown to V1.
#[derive(Copy, Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("unknown block flag bits: {unknown_bits:#06x}")]
pub struct InvalidBlockFlags {
    unknown_bits: RawBlockFlags,
}

impl SszDelegate for BlockFlagsV1 {
    type Delegate = RawBlockFlags;

    fn into_delegate(self) -> Self::Delegate {
        self.0
    }

    fn from_delegate(value: Self::Delegate) -> Result<Self, DecodeError> {
        Self::try_from(value).map_err(|error| DecodeError::BytesInvalid(error.to_string()))
    }
}

impl_ssz_via_delegate!(BlockFlagsV1);

impl TryFrom<RawBlockFlags> for BlockFlagsV1 {
    type Error = InvalidBlockFlags;

    fn try_from(value: RawBlockFlags) -> Result<Self, Self::Error> {
        let unknown_bits = value & !KNOWN_FLAGS;
        if unknown_bits != 0 {
            return Err(InvalidBlockFlags { unknown_bits });
        }

        Ok(Self(value))
    }
}

impl From<BlockFlagsV1> for u16 {
    fn from(value: BlockFlagsV1) -> Self {
        value.0
    }
}

impl BlockFlagsV1 {
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

#[cfg(test)]
mod tests {
    use ssz::view::DecodeView;
    use ssz::{Decode, Encode};
    use strata_identifiers::{Buf32, OLBlockId};

    use crate::OLBlockHeaderV1;

    use super::{BlockFlagsV1, IS_TERMINAL};

    #[test]
    fn rejects_unknown_flag_bits_during_ssz_decoding() {
        let encoded_unknown_flag = 0x0002_u16.to_le_bytes();

        assert!(<BlockFlagsV1 as Decode>::from_ssz_bytes(&encoded_unknown_flag).is_err());
        assert!(<BlockFlagsV1 as DecodeView>::from_ssz_bytes(&encoded_unknown_flag).is_err());
    }

    #[test]
    fn rejects_header_with_unknown_flag_bits_during_ssz_decoding() {
        let header = OLBlockHeaderV1::new(
            0,
            BlockFlagsV1::zero(),
            0,
            0,
            OLBlockId::from(Buf32::zero()),
            Buf32::zero(),
            Buf32::zero(),
            Buf32::zero(),
        );
        let mut encoded_header = header.as_ssz_bytes();
        encoded_header[8..10].copy_from_slice(&0x0002_u16.to_le_bytes());

        assert!(<OLBlockHeaderV1 as Decode>::from_ssz_bytes(&encoded_header).is_err());
    }

    #[test]
    fn rejects_unknown_flag_bits_during_construction() {
        assert!(BlockFlagsV1::try_from(0x0003).is_err());
    }

    #[test]
    fn accepts_and_roundtrips_known_flag_combinations() {
        for raw_flags in [0x0000, IS_TERMINAL] {
            let flags = BlockFlagsV1::try_from(raw_flags).expect("known flags must be valid");
            let encoded = flags.as_ssz_bytes();
            let decoded = <BlockFlagsV1 as Decode>::from_ssz_bytes(&encoded)
                .expect("known flags must decode");

            assert_eq!(decoded, flags);
        }
    }
}
