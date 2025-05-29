use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use crate::buf::Buf32;

/// Structure for `ExecUpdate.input.extra_payload` for EVM EL
#[derive(Debug, BorshSerialize, BorshDeserialize)]
pub struct EVMExtraPayload {
    block_hash: [u8; 32],
}

impl EVMExtraPayload {
    pub const fn new(block_hash: [u8; 32]) -> Self {
        Self { block_hash }
    }

    pub fn block_hash(&self) -> Buf32 {
        self.block_hash.into()
    }
}

/// Generate extra_payload for evm el
pub fn create_evm_extra_payload(block_hash: Buf32) -> Vec<u8> {
    let extra_payload = EVMExtraPayload {
        block_hash: *block_hash.as_ref(),
    };
    borsh::to_vec(&extra_payload).expect("extra_payload vec")
}

#[derive(
    Copy,
    Clone,
    Debug,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    Arbitrary,
    BorshDeserialize,
    BorshSerialize,
    Deserialize,
    Serialize,
)]
pub struct EvmEeBlockCommitment {
    slot: u64,
    blkid: Buf32,
}

impl EvmEeBlockCommitment {
    pub const fn new(slot: u64, blkid: Buf32) -> Self {
        Self { slot, blkid }
    }

    pub fn null() -> Self {
        Self::new(0, Buf32::zero())
    }

    pub const fn slot(&self) -> u64 {
        self.slot
    }

    pub const fn blkid(&self) -> &Buf32 {
        &self.blkid
    }

    pub fn is_null(&self) -> bool {
        self.slot == 0 && self.blkid().is_zero()
    }
}
