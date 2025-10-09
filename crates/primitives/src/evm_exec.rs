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
    pub fn new(block_hash: [u8; 32]) -> Self {
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

// TODO remove this reexport
pub type EvmEeBlockCommitment = strata_identifiers::ExecBlockCommitment;
