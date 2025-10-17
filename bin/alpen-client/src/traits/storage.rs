use std::sync::Arc;

use async_trait::async_trait;
use strata_ee_acct_types::EeAccountState;
use strata_identifiers::OLBlockCommitment;
use tokio::sync::RwLock;

use super::error::StorageError;

#[derive(Debug, Clone)]
/// EE account internal state corresponding to ol Block
pub(crate) struct OlBlockEeAccountState {
    pub ol_block: OLBlockCommitment,
    pub state: EeAccountState,
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
/// Persistence for EE Nodes
pub(crate) trait Storage {
    /// Get EE account internal state corresponding to a given OL slot.
    async fn ee_account_state_for_slot(
        &self,
        ol_slot: u64,
    ) -> Result<Option<OlBlockEeAccountState>, StorageError>;
    /// Get EE account internal state for the highest slot available.
    async fn best_ee_account_state(&self) -> Result<Option<OlBlockEeAccountState>, StorageError>;
    /// Store EE account internal state for next slot.
    async fn store_ee_account_state(
        &self,
        ol_block: &OLBlockCommitment,
        ee_account_state: &EeAccountState,
    ) -> Result<(), StorageError>;
    /// Remove stored EE internal account state for slots > `to_slot`.
    async fn rollback_ee_account_state(&self, to_slot: u64) -> Result<(), StorageError>;
}

#[derive(Debug, Clone, Default)]
pub(crate) struct DummyStorage {
    items: Arc<RwLock<Vec<OlBlockEeAccountState>>>,
}

#[async_trait]
impl Storage for DummyStorage {
    async fn ee_account_state_for_slot(
        &self,
        ol_slot: u64,
    ) -> Result<Option<OlBlockEeAccountState>, StorageError> {
        Ok(self
            .items
            .read()
            .await
            .iter()
            .find(|item| item.ol_block.slot() == ol_slot)
            .cloned())
    }
    async fn best_ee_account_state(&self) -> Result<Option<OlBlockEeAccountState>, StorageError> {
        Ok(self.items.read().await.last().cloned())
    }
    async fn store_ee_account_state(
        &self,
        ol_block: &OLBlockCommitment,
        ee_account_state: &EeAccountState,
    ) -> Result<(), StorageError> {
        if let Some(last_item) = self.items.read().await.last() {
            if last_item.ol_block.slot() + 1 != ol_block.slot() {
                return Err(StorageError::MissingSlot {
                    attempted_slot: ol_block.slot(),
                    last_slot: last_item.ol_block.slot(),
                });
            }
        }
        self.items.write().await.push(OlBlockEeAccountState {
            ol_block: *ol_block,
            state: ee_account_state.clone(),
        });
        Ok(())
    }
    async fn rollback_ee_account_state(&self, to_slot: u64) -> Result<(), StorageError> {
        let mut items = self.items.write().await;
        let Some(base_idx) = items.first().map(|item| item.ol_block.slot()) else {
            return Ok(());
        };
        let truncate_idx = to_slot.saturating_sub(base_idx);

        items.truncate(truncate_idx as usize);

        Ok(())
    }
}
