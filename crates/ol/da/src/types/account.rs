//! Account diff types.

use strata_acct_types::BitcoinAmount;
use strata_codec::{Codec, CodecError, Decoder, Encoder};
use strata_da_framework::{BitSeqReader, BitSeqWriter, CompoundMember, DaRegister, DaWrite};
use strata_identifiers::AccountTypeId;

use super::snark::SnarkAccountDiff;

/// Per-account diff keyed by account type.
///
/// This enum allows different diff structures for each account type, making
/// the system extensible for future account types.
#[derive(Debug)]
pub enum AccountDiff {
    /// Empty account diff, only balance changes.
    Empty {
        /// Balance register diff.
        balance: DaRegister<BitcoinAmount>,
    },

    /// Snark account diff, balance and snark-specific state.
    Snark {
        /// Balance register diff.
        balance: DaRegister<BitcoinAmount>,

        /// Snark state diff.
        snark: SnarkAccountDiff,
    },
}

impl Default for AccountDiff {
    fn default() -> Self {
        Self::Empty {
            balance: DaRegister::new_unset(),
        }
    }
}

impl AccountDiff {
    /// Creates a new empty account diff.
    pub fn new_empty(balance: DaRegister<BitcoinAmount>) -> Self {
        Self::Empty { balance }
    }

    /// Creates a new snark account diff.
    pub fn new_snark(balance: DaRegister<BitcoinAmount>, snark: SnarkAccountDiff) -> Self {
        Self::Snark { balance, snark }
    }

    /// Returns the account type ID for this diff.
    pub fn type_id(&self) -> AccountTypeId {
        match self {
            Self::Empty { .. } => AccountTypeId::Empty,
            Self::Snark { .. } => AccountTypeId::Snark,
        }
    }

    /// Returns the balance diff, regardless of account type.
    pub fn balance(&self) -> &DaRegister<BitcoinAmount> {
        match self {
            Self::Empty { balance } | Self::Snark { balance, .. } => balance,
        }
    }

    pub fn is_default(&self) -> bool {
        match self {
            Self::Empty { balance } => DaWrite::is_default(balance),
            Self::Snark { balance, snark } => {
                DaWrite::is_default(balance) && CompoundMember::is_default(snark)
            }
        }
    }
}

impl Codec for AccountDiff {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        // Encode account type ID first
        let type_id: u8 = match self {
            Self::Empty { .. } => 0,
            Self::Snark { .. } => 1,
        };
        type_id.encode(enc)?;

        match self {
            Self::Empty { balance } => {
                let mut bitw = BitSeqWriter::<u8>::new();
                bitw.prepare_member(balance);
                bitw.mask().encode(enc)?;
                if !DaWrite::is_default(balance) {
                    CompoundMember::encode_set(balance, enc)?;
                }
            }
            Self::Snark { balance, snark } => {
                let mut bitw = BitSeqWriter::<u8>::new();
                bitw.prepare_member(balance);
                bitw.prepare_member(snark);
                bitw.mask().encode(enc)?;
                if !DaWrite::is_default(balance) {
                    CompoundMember::encode_set(balance, enc)?;
                }
                if !CompoundMember::is_default(snark) {
                    CompoundMember::encode_set(snark, enc)?;
                }
            }
        }
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let type_id = u8::decode(dec)?;
        let bitmask = u8::decode(dec)?;

        match type_id {
            0 => {
                let mut bitr = BitSeqReader::from_mask(bitmask);
                let balance = bitr.decode_next_member::<DaRegister<BitcoinAmount>>(dec)?;
                Ok(Self::Empty { balance })
            }
            1 => {
                let mut bitr = BitSeqReader::from_mask(bitmask);
                let balance = bitr.decode_next_member::<DaRegister<BitcoinAmount>>(dec)?;
                let snark = bitr.decode_next_member::<SnarkAccountDiff>(dec)?;
                Ok(Self::Snark { balance, snark })
            }
            _ => Err(CodecError::InvalidVariant("account_type_id")),
        }
    }
}
