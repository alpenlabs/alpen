//! Replay pre-state snapshot types.

use std::collections::BTreeMap;

use alloy_primitives::{Bytes, B256};
use alpen_reth_statediff::StateReconstructorPreState;
use serde::{Deserialize, Serialize};
use strata_identifiers::Buf32;

/// Explicit starting state for partial-range replay.
///
/// The snapshot represents the EE state immediately before
/// [`ReplayPreStateSnapshot::next_update_seq_no`]. It carries the canonical
/// reconstructor prestate plus bytecode preimages for accounts that existed
/// before the replayed DA range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayPreStateSnapshot {
    expected_state_root: Buf32,
    next_update_seq_no: u64,
    last_applied_block_num: u64,
    reconstructor_prestate: StateReconstructorPreState,
    bytecodes: BTreeMap<B256, Bytes>,
}

impl ReplayPreStateSnapshot {
    /// Creates an explicit starting state snapshot for partial replay.
    pub fn new(
        expected_state_root: Buf32,
        next_update_seq_no: u64,
        last_applied_block_num: u64,
        reconstructor_prestate: StateReconstructorPreState,
        bytecodes: BTreeMap<B256, Bytes>,
    ) -> Self {
        Self {
            expected_state_root,
            next_update_seq_no,
            last_applied_block_num,
            reconstructor_prestate,
            bytecodes,
        }
    }

    /// Returns the expected root of the pre-state.
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

    /// Returns the reconstructor prestate used to initialize replay.
    pub fn reconstructor_prestate(&self) -> &StateReconstructorPreState {
        &self.reconstructor_prestate
    }

    /// Returns bytecode preimages for accounts that existed before replay.
    pub fn bytecodes(&self) -> &BTreeMap<B256, Bytes> {
        &self.bytecodes
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, U256};
    use strata_mpt::{StateAccount, EMPTY_ROOT};

    use super::*;

    #[test]
    fn replay_pre_state_snapshot_json_roundtrips() {
        let address = Address::from([0x11; 20]);
        let slot_key = U256::from(1);
        let slot_value = U256::from(2);
        let account = StateAccount {
            nonce: 7,
            balance: U256::from(100),
            storage_root: EMPTY_ROOT,
            code_hash: B256::from([0x22; 32]),
        };
        let reconstructor_prestate = StateReconstructorPreState::new(
            BTreeMap::from([(address, account)]),
            BTreeMap::from([(address, BTreeMap::from([(slot_key, slot_value)]))]),
        );
        let snapshot = ReplayPreStateSnapshot::new(
            Buf32::from([0x33; 32]),
            9,
            123,
            reconstructor_prestate,
            BTreeMap::from([(B256::from([0x44; 32]), Bytes::from_static(b"bytecode"))]),
        );

        let encoded = serde_json::to_string(&snapshot).expect("snapshot must serialize");
        let decoded: ReplayPreStateSnapshot =
            serde_json::from_str(&encoded).expect("snapshot must deserialize");

        assert_eq!(decoded, snapshot);
    }
}
