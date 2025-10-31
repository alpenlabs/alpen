use std::sync::Arc;

use async_trait::async_trait;
use strata_acct_types::Hash;
use strata_ee_acct_types::EeAccountState;
use strata_identifiers::{OLBlockCommitment, OLBlockId};
use tokio::sync::RwLock;

use super::error::StorageError;

#[derive(Debug, Clone)]
/// EE account internal state corresponding to ol Block
pub(crate) struct EeAccountStateAtBlock {
    ol_block: OLBlockCommitment,
    state: EeAccountState,
}

impl EeAccountStateAtBlock {
    pub(crate) fn new(ol_block: OLBlockCommitment, state: EeAccountState) -> Self {
        Self { ol_block, state }
    }

    pub(crate) fn ol_block(&self) -> &OLBlockCommitment {
        &self.ol_block
    }
    pub(crate) fn ee_state(&self) -> &EeAccountState {
        &self.state
    }

    #[expect(dead_code, reason = "wip")]
    pub(crate) fn ol_slot(&self) -> u64 {
        self.ol_block.slot()
    }
    #[expect(dead_code, reason = "wip")]
    pub(crate) fn ol_blockid(&self) -> &OLBlockId {
        self.ol_block.blkid()
    }
    pub(crate) fn last_exec_blkid(&self) -> Hash {
        self.state.last_exec_blkid()
    }
}

pub(crate) enum OLBlockOrSlot<'a> {
    Block(&'a OLBlockId),
    Slot(u64),
}

impl<'a> From<&'a OLBlockId> for OLBlockOrSlot<'a> {
    fn from(value: &'a OLBlockId) -> Self {
        Self::Block(value)
    }
}

impl From<u64> for OLBlockOrSlot<'_> {
    fn from(value: u64) -> Self {
        OLBlockOrSlot::Slot(value)
    }
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
/// Persistence for EE Nodes
pub(crate) trait Storage {
    /// Get EE account internal state corresponding to a given OL slot.
    async fn ee_account_state<'a>(
        &self,
        block_or_slot: OLBlockOrSlot<'a>,
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
    #[expect(dead_code, reason = "will be used in reorg handling")]
    async fn rollback_ee_account_state(&self, to_slot: u64) -> Result<(), StorageError>;
}

#[derive(Debug, Clone, Default)]
pub(crate) struct DummyStorage {
    items: Arc<RwLock<Vec<EeAccountStateAtBlock>>>,
}

#[async_trait]
impl Storage for DummyStorage {
    async fn ee_account_state<'a>(
        &self,
        block_or_slot: OLBlockOrSlot<'a>,
    ) -> Result<Option<EeAccountStateAtBlock>, StorageError> {
        Ok(self
            .items
            .read()
            .await
            .iter()
            .find(|item| match block_or_slot {
                OLBlockOrSlot::Block(blockid) => item.ol_block.blkid() == blockid,
                OLBlockOrSlot::Slot(slot) => item.ol_block.slot() == slot,
            })
            .cloned())
    }
    async fn best_ee_account_state(&self) -> Result<Option<EeAccountStateAtBlock>, StorageError> {
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
        self.items.write().await.push(EeAccountStateAtBlock {
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
