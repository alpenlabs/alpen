#[cfg(feature = "arbitrary")]
use arbitrary::Arbitrary;
#[cfg(feature = "borsh")]
use borsh::{BorshDeserialize, BorshSerialize};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::buf::Buf32;

/// Alias to [`Buf32`] used as a universal hash type in EE.
pub type Hash = Buf32;

/// Structure for `ExecUpdate.input.extra_payload` for EVM EL
#[derive(Debug)]
#[cfg_attr(feature = "borsh", derive(BorshSerialize, BorshDeserialize))]
pub struct EVMExtraPayload {
    block_hash: [u8; 32],
}

impl EVMExtraPayload {
    pub fn new(block_hash: [u8; 32]) -> Self {
        Self { block_hash }
    }

    pub fn block_hash(&self) -> Buf32 {
        self.block_hash.into()
    }
}

pub fn create_evm_extra_payload(block_hash: Buf32) -> Vec<u8> {
    block_hash.0.to_vec()
}

/// Commitment to an execution block, containing slot and block ID.
///
/// This type was previously named `EvmEeBlockCommitment` but has been renamed
/// to `ExecBlockCommitment` to be more generic and not tied to EVM.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "borsh", derive(BorshDeserialize, BorshSerialize))]
#[cfg_attr(feature = "arbitrary", derive(Arbitrary))]
pub struct ExecBlockCommitment {
    slot: u64,
    blkid: Buf32,
}

impl ExecBlockCommitment {
    pub fn new(slot: u64, blkid: Buf32) -> Self {
        Self { slot, blkid }
    }

    pub fn null() -> Self {
        Self::new(0, Buf32::zero())
    }

    pub fn slot(&self) -> u64 {
        self.slot
    }

    pub fn blkid(&self) -> &Buf32 {
        &self.blkid
    }

    pub fn is_null(&self) -> bool {
        self.slot == 0 && self.blkid().is_zero()
    }
}

/// Alias for backward compatibility
pub type EvmEeBlockCommitment = ExecBlockCommitment;
