use async_trait::async_trait;
use strata_ee_acct_types::EeAccountState;
use strata_identifiers::{OLBlockCommitment, OLBlockId};

use super::StorageError;
use crate::EeAccountStateAtBlock;

/// Identifies an OL block either by block ID or slot number.
#[derive(Debug)]
pub enum OLBlockOrSlot {
    /// Identifies by block ID.
    Block(OLBlockId),
    /// Identifies by slot number.
    Slot(u64),
}

impl From<OLBlockId> for OLBlockOrSlot {
    fn from(value: OLBlockId) -> Self {
        Self::Block(value)
    }
}

impl From<&OLBlockId> for OLBlockOrSlot {
    fn from(value: &OLBlockId) -> Self {
        Self::Block(*value)
    }
}

impl From<u64> for OLBlockOrSlot {
    fn from(value: u64) -> Self {
        OLBlockOrSlot::Slot(value)
    }
}

#[cfg_attr(feature = "test-utils", mockall::automock)]
#[async_trait]
/// Persistence for EE Nodes
pub trait Storage {
    /// Get EE account internal state corresponding to a given OL slot.
    async fn ee_account_state(
        &self,
        block_or_slot: OLBlockOrSlot,
    ) -> Result<Option<EeAccountStateAtBlock>, StorageError>;

    /// Get EE account internal state for the highest slot available.
    async fn best_ee_account_state(&self) -> Result<Option<EeAccountStateAtBlock>, StorageError>;

    /// Store EE account internal state for next slot.
    async fn store_ee_account_state(
        &self,
        ol_block: &OLBlockCommitment,
        ee_account_state: &EeAccountState,
    ) -> Result<(), StorageError>;

    /// Remove stored EE internal account state for slots > `to_slot`.
    async fn rollback_ee_account_state(&self, to_slot: u64) -> Result<(), StorageError>;
}

/// Macro to instantiate all Storage tests for a given storage setup.
#[cfg(feature = "test-utils")]
#[macro_export]
macro_rules! storage_tests {
    ($setup_expr:expr) => {
        #[tokio::test]
        async fn test_store_and_get_ee_account_state() {
            let storage = $setup_expr;
            $crate::storage_test_fns::test_store_and_get_ee_account_state(&storage).await;
        }

        #[tokio::test]
        async fn test_sequential_slots() {
            let storage = $setup_expr;
            $crate::storage_test_fns::test_sequential_slots(&storage).await;
        }

        #[tokio::test]
        async fn test_null_block_rejected() {
            let storage = $setup_expr;
            $crate::storage_test_fns::test_null_block_rejected(&storage).await;
        }

        #[tokio::test]
        async fn test_rollback_ee_account_state() {
            let storage = $setup_expr;
            $crate::storage_test_fns::test_rollback_ee_account_state(&storage).await;
        }

        #[tokio::test]
        async fn test_empty_storage() {
            let storage = $setup_expr;
            $crate::storage_test_fns::test_empty_storage(&storage).await;
        }

        #[tokio::test]
        async fn test_rollback_empty_storage() {
            let storage = $setup_expr;
            $crate::storage_test_fns::test_rollback_empty_storage(&storage).await;
        }

        #[tokio::test]
        async fn test_sequential_writes_and_retrieval() {
            let storage = $setup_expr;
            $crate::storage_test_fns::test_sequential_writes_and_retrieval(&storage).await;
        }
    };
}

#[cfg(feature = "test-utils")]
pub mod tests {
    use strata_acct_types::BitcoinAmount;
    use strata_ee_acct_types::EeAccountState;
    use strata_identifiers::{Buf32, OLBlockCommitment, OLBlockId};

    use super::*;

    fn create_test_block_id(value: u8) -> OLBlockId {
        OLBlockId::from(Buf32::new([value; 32]))
    }

    fn create_test_ee_account_state() -> EeAccountState {
        EeAccountState::new([0u8; 32], BitcoinAmount::ZERO, Vec::new(), Vec::new())
    }

    /// Test storing and retrieving EE account state.
    pub async fn test_store_and_get_ee_account_state(storage: &impl Storage) {
        // Create test data
        let slot = 100u64;
        let block_id = create_test_block_id(1);
        let ol_block = OLBlockCommitment::new(slot, block_id);
        let ee_account_state = create_test_ee_account_state();

        // Store the account state
        storage
            .store_ee_account_state(&ol_block, &ee_account_state)
            .await
            .unwrap();

        // Retrieve by block ID
        let retrieved = storage
            .ee_account_state(OLBlockOrSlot::Block(block_id))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(retrieved.ol_block(), &ol_block);
        assert_eq!(retrieved.ee_state(), &ee_account_state);

        // Retrieve by slot
        let retrieved_by_slot = storage
            .ee_account_state(OLBlockOrSlot::Slot(slot))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(retrieved_by_slot.ol_block().blkid(), &block_id);

        // Retrieve best state
        let best = storage.best_ee_account_state().await.unwrap().unwrap();
        assert_eq!(best.ol_block(), &ol_block);
        assert_eq!(best.ee_state(), &ee_account_state);
    }

    /// Test sequential slot enforcement.
    pub async fn test_sequential_slots(storage: &impl Storage) {
        // First write should succeed at any slot
        let slot1 = 100u64;
        let block_id1 = create_test_block_id(1);
        let ol_block1 = OLBlockCommitment::new(slot1, block_id1);
        let ee_account_state1 = create_test_ee_account_state();

        storage
            .store_ee_account_state(&ol_block1, &ee_account_state1)
            .await
            .unwrap();

        // Next write must be at slot1 + 1
        let slot2 = slot1 + 1;
        let block_id2 = create_test_block_id(2);
        let ol_block2 = OLBlockCommitment::new(slot2, block_id2);
        let ee_account_state2 = create_test_ee_account_state();

        storage
            .store_ee_account_state(&ol_block2, &ee_account_state2)
            .await
            .unwrap();

        // Writing to a non-sequential slot should fail
        let slot_skip = slot2 + 2; // Skip slot2 + 1
        let block_id_skip = create_test_block_id(3);
        let ol_block_skip = OLBlockCommitment::new(slot_skip, block_id_skip);
        let ee_account_state_skip = create_test_ee_account_state();

        let result = storage
            .store_ee_account_state(&ol_block_skip, &ee_account_state_skip)
            .await;
        assert!(result.is_err());
    }

    /// Test null block rejection.
    pub async fn test_null_block_rejected(storage: &impl Storage) {
        let null_block = OLBlockCommitment::null();
        let ee_account_state = create_test_ee_account_state();

        let result = storage
            .store_ee_account_state(&null_block, &ee_account_state)
            .await;
        assert!(result.is_err());
        // The underlying database layer should reject null blocks
    }

    /// Test rollback functionality.
    pub async fn test_rollback_ee_account_state(storage: &impl Storage) {
        // Create a sequence of states
        let slots = [100u64, 101, 102, 103, 104];
        let mut block_ids = Vec::new();

        for slot in slots {
            let block_id = create_test_block_id(slot as u8);
            block_ids.push(block_id);
            let ol_block = OLBlockCommitment::new(slot, block_id);
            let ee_account_state = create_test_ee_account_state();

            storage
                .store_ee_account_state(&ol_block, &ee_account_state)
                .await
                .unwrap();
        }

        // Rollback to slot 101
        storage.rollback_ee_account_state(101).await.unwrap();

        // Slots 100 and 101 should still exist
        assert!(storage
            .ee_account_state(OLBlockOrSlot::Slot(100))
            .await
            .unwrap()
            .is_some());
        assert!(storage
            .ee_account_state(OLBlockOrSlot::Slot(101))
            .await
            .unwrap()
            .is_some());

        // Slots 102, 103, 104 should be gone (StateNotFound error expected)
        assert!(matches!(
            storage.ee_account_state(OLBlockOrSlot::Slot(102)).await,
            Err(StorageError::StateNotFound(102))
        ));
        assert!(matches!(
            storage.ee_account_state(OLBlockOrSlot::Slot(103)).await,
            Err(StorageError::StateNotFound(103))
        ));
        assert!(matches!(
            storage.ee_account_state(OLBlockOrSlot::Slot(104)).await,
            Err(StorageError::StateNotFound(104))
        ));

        // Best state should be at slot 101
        let best = storage.best_ee_account_state().await.unwrap().unwrap();
        assert_eq!(best.ol_block().slot(), 101);
    }

    /// Test empty storage behavior.
    pub async fn test_empty_storage(storage: &impl Storage) {
        // Best state should be None on empty storage
        let best = storage.best_ee_account_state().await.unwrap();
        assert!(best.is_none());

        // Getting non-existent block ID should return None
        let block_id = create_test_block_id(1);
        let state = storage
            .ee_account_state(OLBlockOrSlot::Block(block_id))
            .await
            .unwrap();
        assert!(state.is_none());

        // Getting non-existent slot should return StateNotFound error
        assert!(matches!(
            storage.ee_account_state(OLBlockOrSlot::Slot(999)).await,
            Err(StorageError::StateNotFound(999))
        ));
    }

    /// Test rollback on empty storage.
    pub async fn test_rollback_empty_storage(storage: &impl Storage) {
        // Rollback on empty storage should succeed (no-op)
        let result = storage.rollback_ee_account_state(100).await;
        assert!(result.is_ok());
    }

    /// Test multiple sequential writes and retrieval.
    pub async fn test_sequential_writes_and_retrieval(storage: &impl Storage) {
        let num_blocks = 10;
        let start_slot = 200u64;

        // Write multiple sequential blocks
        for i in 0..num_blocks {
            let slot = start_slot + i;
            let block_id = create_test_block_id(i as u8);
            let ol_block = OLBlockCommitment::new(slot, block_id);
            let ee_account_state = create_test_ee_account_state();

            storage
                .store_ee_account_state(&ol_block, &ee_account_state)
                .await
                .unwrap();
        }

        // Verify all blocks can be retrieved
        for i in 0..num_blocks {
            let slot = start_slot + i;
            let expected_block_id = create_test_block_id(i as u8);

            let state = storage
                .ee_account_state(OLBlockOrSlot::Slot(slot))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(state.ol_block().slot(), slot);
            assert_eq!(state.ol_block().blkid(), &expected_block_id);
        }

        // Best state should be the last one
        let best = storage.best_ee_account_state().await.unwrap().unwrap();
        assert_eq!(best.ol_block().slot(), start_slot + num_blocks - 1);
    }
}
