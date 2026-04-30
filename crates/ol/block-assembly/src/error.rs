//! Error types for block assembly operations.

use std::error::Error;

use strata_acct_types::AcctError;
use strata_db_types::errors::DbError;
use strata_identifiers::{AccountId, Epoch, Hash, OLBlockCommitment, OLBlockId};
use strata_ledger_types::StateError;
use strata_ol_chain_types_new::ChainTypesError;
use strata_ol_mempool::OLMempoolError;
use strata_ol_stf::ExecError;

/// Errors that can occur during block assembly operations.
#[derive(Debug, thiserror::Error)]
pub enum BlockAssemblyError {
    /// Invalid L1 block range where `from_block` height > `to_block` height.
    #[error("invalid L1 block height range (from {from_height} to {to_height})")]
    InvalidRange { from_height: u64, to_height: u64 },

    /// L1 header claim hash does not match MMR entry.
    #[error("L1 header hash mismatch at index {idx} (expected {expected}, got {actual})")]
    L1HeaderHashMismatch {
        idx: u64,
        expected: Hash,
        actual: Hash,
    },

    /// Inbox message hash does not match MMR entry.
    #[error(
        "inbox hash mismatch at index {idx} for account {account_id} (expected {expected}, got {actual})"
    )]
    InboxEntryHashMismatch {
        idx: u64,
        account_id: AccountId,
        expected: Hash,
        actual: Hash,
    },

    /// Account not found when validating transaction.
    #[error("account not found: {0}")]
    AccountNotFound(AccountId),

    /// Inbox MMR proof count mismatch.
    #[error("inbox MMR proof count mismatch (expected {expected}, got {got})")]
    InboxProofCountMismatch { expected: usize, got: usize },

    /// Unknown template ID (template not found in pending templates).
    #[error("no pending template found for id: {0}")]
    UnknownTemplateId(OLBlockId),

    /// Block not found in db.
    #[error("block not found in db: {0}")]
    BlockNotFound(OLBlockId),

    /// Parent state not found in db.
    #[error("parent state not found in db: {0}")]
    ParentStateNotFound(OLBlockCommitment),

    /// No mapping found in parent block ID -> template ID cache.
    #[error("no pending template found for parent id: {0}")]
    NoPendingTemplateForParent(OLBlockId),

    /// Invalid signature for block template completion.
    #[error("invalid signature for template: {0}")]
    InvalidSignature(OLBlockId),

    /// Block timestamp is too early (violates minimum block time).
    #[error("block timestamp too early: {0}")]
    TimestampTooEarly(u64),

    /// Invalid accumulator claim in transaction.
    #[error("invalid accumulator claim: {0}")]
    InvalidAccumulatorClaim(String),

    #[error("too many accumulator claims")]
    TooManyClaims,

    /// Attempted to build genesis block via block assembly.
    /// Genesis must be created via `init_ol_genesis` at node startup.
    #[error("cannot build genesis block via block assembly")]
    CannotBuildGenesis,

    /// Genesis epoch has no boundary.
    #[error("genesis epoch has no boundary")]
    GenesisEpochNoBoundary,

    /// Epoch boundary block does not satisfy expected terminal/epoch properties.
    #[error(
        "invalid epoch boundary at {blkid}: expected prev epoch {expected_prev_epoch}, got {got_epoch}, terminal={is_terminal}"
    )]
    InvalidEpochBoundary {
        blkid: OLBlockId,
        expected_prev_epoch: Epoch,
        got_epoch: Epoch,
        is_terminal: bool,
    },

    /// Epoch boundary state not found in db.
    #[error("epoch boundary state not found in db: {0}")]
    EpochBoundaryStateNotFound(OLBlockCommitment),

    /// Request channel closed (service shutdown).
    #[error("request channel closed")]
    RequestChannelClosed,

    /// Response channel closed (oneshot sender dropped).
    #[error("response channel closed")]
    ResponseChannelClosed,

    /// Database operation failed.
    #[error("db: {0}")]
    Db(#[from] DbError),

    /// Various account errors.
    #[error("acct: {0}")]
    Acct(#[from] AcctError),

    /// State accessor error.
    #[error("state: {0}")]
    State(#[from] StateError),

    /// Chain types construction failed.
    #[error("chain types: {0}")]
    ChainTypes(#[from] ChainTypesError),

    /// Mempool operation failed.
    #[error("mempool: {0}")]
    Mempool(#[from] OLMempoolError),

    /// State provider operation failed.
    #[error("state provider: {0}")]
    StateProvider(#[source] Box<dyn Error + Send + Sync>),

    /// Block construction/execution failed.
    #[error("block construction: {0}")]
    BlockConstruction(#[from] ExecError),

    /// Snark account update failed pre-validation during proof indexing.
    ///
    /// This wraps errors from `verify_snark_acct_update_proofs` when using the
    /// `TxProofIndexer` to discover needed proofs.
    #[error("snark update pre-validation: {0}")]
    SnarkUpdatePreValidation(ExecError),

    /// Other unexpected error.
    #[error("other: {0}")]
    Other(String),
}
