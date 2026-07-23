//! Replay state snapshot types.

use std::collections::BTreeMap;

use alloy_primitives::{Bytes, B256};
use alpen_reth_statediff::EthereumStateExt;
use rsp_mpt::EthereumState;
use serde::{Deserialize, Serialize};
use strata_identifiers::Buf32;

/// Current JSON snapshot artifact version.
///
/// Version `1` is the initial schema. Bump this when the serialized shape
/// changes, including upstream [`EthereumState`] serde changes.
pub const SNAPSHOT_FORMAT_VERSION: u32 = 1;

/// Explicit replay state for partial-range replay.
///
/// The snapshot represents the EE state immediately before
/// [`ReplayStateSnapshot::next_update_seq_no`]. It carries the Ethereum
/// state plus bytecode preimages for accounts that existed before the
/// replayed DA range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayStateSnapshot {
    version: u32,
    expected_state_root: Buf32,
    next_update_seq_no: u64,
    last_applied_block_num: u64,
    ethereum_state: EthereumState,
    bytecodes: BTreeMap<B256, Bytes>,
}

impl ReplayStateSnapshot {
    /// Creates an explicit state snapshot for partial replay.
    pub fn new(
        next_update_seq_no: u64,
        last_applied_block_num: u64,
        ethereum_state: EthereumState,
        bytecodes: BTreeMap<B256, Bytes>,
    ) -> Self {
        let expected_state_root = ethereum_state.state_root_buf32();
        Self {
            version: SNAPSHOT_FORMAT_VERSION,
            expected_state_root,
            next_update_seq_no,
            last_applied_block_num,
            ethereum_state,
            bytecodes,
        }
    }

    /// Returns the snapshot artifact version.
    pub fn version(&self) -> u32 {
        self.version
    }

    /// Returns the expected root of the state.
    pub fn expected_state_root(&self) -> Buf32 {
        self.expected_state_root
    }

    /// Returns the first update sequence number accepted by this snapshot.
    pub fn next_update_seq_no(&self) -> u64 {
        self.next_update_seq_no
    }

    /// Returns the last EVM block number applied before this snapshot.
    pub fn last_applied_block_num(&self) -> u64 {
        self.last_applied_block_num
    }

    /// Returns the Ethereum state used to initialize replay.
    pub fn ethereum_state(&self) -> &EthereumState {
        &self.ethereum_state
    }

    /// Consumes the snapshot and returns its Ethereum state.
    pub fn into_ethereum_state(self) -> EthereumState {
        self.ethereum_state
    }

    /// Returns bytecode preimages for accounts that existed before replay.
    pub fn bytecodes(&self) -> &BTreeMap<B256, Bytes> {
        &self.bytecodes
    }
}

#[cfg(test)]
mod tests {
    use alpen_reth_statediff::{
        ethereum_state_from_genesis_accounts,
        test_utils::{addr, slot, value as storage_or_balance_value},
        EthereumStateExt, GenesisAccount,
    };

    use super::*;

    #[test]
    fn test_snapshot_json_roundtrip() {
        let ethereum_state = ethereum_state_from_genesis_accounts([(
            addr(0x11),
            build_genesis_account_with_storage(7, 100, 1, 2),
        )])
        .expect("genesis state builds");
        let snapshot = ReplayStateSnapshot::new(
            9,
            123,
            ethereum_state,
            BTreeMap::from([(B256::from([0x44; 32]), Bytes::from_static(b"bytecode"))]),
        );

        let encoded = serde_json::to_string(&snapshot).expect("snapshot must serialize");
        let decoded: ReplayStateSnapshot =
            serde_json::from_str(&encoded).expect("snapshot must deserialize");

        assert_eq!(decoded, snapshot);
        assert_eq!(decoded.version(), SNAPSHOT_FORMAT_VERSION);
        assert_eq!(
            decoded.ethereum_state().state_root_buf32(),
            decoded.expected_state_root()
        );
    }

    fn build_genesis_account_with_storage(
        nonce: u64,
        balance: u64,
        slot_key: u64,
        slot_value: u64,
    ) -> GenesisAccount {
        let mut storage = BTreeMap::new();
        storage.insert(
            B256::from(slot(slot_key).to_be_bytes::<32>()),
            B256::from(storage_or_balance_value(slot_value).to_be_bytes::<32>()),
        );

        GenesisAccount {
            nonce: Some(nonce),
            balance: storage_or_balance_value(balance),
            code: None,
            storage: Some(storage),
            private_key: None,
        }
    }
}
