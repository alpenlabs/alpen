//! Provider trait for the OL RPC server.
//!
//! Abstracts the storage, chain status, and mempool dependencies so the
//! server implementation can be tested with lightweight mock providers.

use std::sync::Arc;

use async_trait::async_trait;
use strata_asm_common::AsmManifest;
use strata_checkpoint_types::EpochSummary;
use strata_csm_types::CheckpointL1Ref;
use strata_db_types::{types::AccountExtraDataEntry, DbResult};
use strata_identifiers::{AccountId, Epoch, L1Height, OLBlockId, OLTxId};
use strata_ol_chain_types_new::{OLBlock, OLTransaction};
use strata_ol_mempool::OLMempoolResult;
use strata_ol_state_types::OLState;
use strata_primitives::{epoch::EpochCommitment, nonempty_vec::NonEmptyVec, OLBlockCommitment};
use strata_status::OLSyncStatus;

/// Extra data associated with an account at a given epoch.
pub type AccountExtraData = NonEmptyVec<AccountExtraDataEntry>;

/// Provides all data access needed by the OL RPC server.
#[async_trait]
pub trait OLRpcProvider: Send + Sync + 'static {
    /// Get the canonical block commitment at the given slot height.
    async fn get_canonical_block_at(&self, height: u64) -> DbResult<Option<OLBlockCommitment>>;

    /// Get block data by block ID.
    async fn get_block_data(&self, id: OLBlockId) -> DbResult<Option<OLBlock>>;

    /// Get the top-level OL state at a given block commitment.
    async fn get_toplevel_ol_state(
        &self,
        commitment: OLBlockCommitment,
    ) -> DbResult<Option<Arc<OLState>>>;

    /// Get the canonical epoch commitment for the given epoch.
    async fn get_canonical_epoch_commitment_at(
        &self,
        epoch: Epoch,
    ) -> DbResult<Option<EpochCommitment>>;

    /// Get OL epoch summary by epoch commitment.
    async fn get_epoch_summary(
        &self,
        commitment: EpochCommitment,
    ) -> DbResult<Option<EpochSummary>>;

    /// Get OL checkpoint L1 ref by epoch commitment.
    async fn get_checkpoint_l1_ref(
        &self,
        commitment: EpochCommitment,
    ) -> DbResult<Option<CheckpointL1Ref>>;

    /// Get extra data entries for an account at a given epoch.
    async fn get_account_extra_data(
        &self,
        key: (AccountId, Epoch),
    ) -> DbResult<Option<AccountExtraData>>;

    /// Get the epoch in which an account was created.
    async fn get_account_creation_epoch(&self, account_id: AccountId) -> DbResult<Option<Epoch>>;

    /// Get the L1 block manifest at a given height.
    async fn get_block_manifest_at_height(&self, height: L1Height)
        -> DbResult<Option<AsmManifest>>;

    /// Get current OL chain sync status.
    fn get_ol_sync_status(&self) -> Option<OLSyncStatus>;

    /// Get current L1 tip height.
    fn get_l1_tip_height(&self) -> Option<L1Height>;

    /// Submit a transaction to the mempool.
    async fn submit_transaction(&self, tx: OLTransaction) -> OLMempoolResult<OLTxId>;
}
