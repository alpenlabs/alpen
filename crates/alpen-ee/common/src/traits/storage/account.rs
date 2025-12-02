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
