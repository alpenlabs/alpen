use async_trait::async_trait;
use strata_identifiers::{Epoch, OLBlockCommitment};
use strata_snark_acct_types::{MessageEntry, ProofState, Seqno, SnarkAccountUpdate};
use thiserror::Error;

use crate::{OLChainStatus, OLEpochSummary};

/// Client interface for interacting with the OL chain.
///
/// Provides methods to view OL Chain data required by an alpen EE fullnode.
#[cfg_attr(feature = "test-utils", mockall::automock)]
#[async_trait]
pub trait OLClient: Sized + Send + Sync {
    /// Returns the current status of the OL chain.
    ///
    /// Includes the latest, confirmed, and finalized block commitments.
    async fn chain_status(&self) -> Result<OLChainStatus, OLClientError>;

    /// Retrieves epoch commitment and update operations for the specified epoch.
    async fn epoch_summary(&self, epoch: Epoch) -> Result<OLEpochSummary, OLClientError>;
}

/// Returns the current status of the OL chain.
///
/// This is a checked version of [`OLClient::chain_status`] that validates
/// the slot numbers of latest >= confirmed >= finalized
pub async fn chain_status_checked(client: &impl OLClient) -> Result<OLChainStatus, OLClientError> {
    let status = client.chain_status().await?;
    if status.finalized.last_slot() > status.confirmed.last_slot()
        || status.confirmed.last_slot() > status.latest.slot()
    {
        return Err(OLClientError::InvalidChainStatusSlotOrder {
            latest: status.latest.slot(),
            confirmed: status.confirmed.last_slot(),
            finalized: status.finalized.last_slot(),
        });
    }
    Ok(status)
}

#[derive(Debug, Clone)]
pub struct OLBlockData {
    pub commitment: OLBlockCommitment,
    pub inbox_messages: Vec<MessageEntry>,
    pub next_inbox_msg_idx: u64,
}

#[derive(Debug, Clone)]
pub struct OLAccountState {
    pub seq_no: Seqno,
    pub proof_state: ProofState,
}

/// Client interface for sequencer-specific OL chain interactions.
///
/// Extends the base OL client functionality with methods needed by the sequencer
/// to read inbox messages and submit state updates to the OL chain.
#[cfg_attr(feature = "test-utils", mockall::automock)]
#[async_trait]
pub trait SequencerOLClient {
    /// Returns the current status of the OL chain.
    ///
    /// Includes the latest, confirmed, and finalized block commitments.
    async fn chain_status(&self) -> Result<OLChainStatus, OLClientError>;

    /// Retrieves inbox messages for the specified slot range (inclusive).
    ///
    /// Returns block data containing commitments and inbox messages for each slot
    /// from `min_slot` to `max_slot`.
    async fn get_inbox_messages(
        &self,
        min_slot: u64,
        max_slot: u64,
    ) -> Result<Vec<OLBlockData>, OLClientError>;

    /// Retrieves latest account state in the OL Chain for this account.
    async fn get_latest_account_state(&self) -> Result<OLAccountState, OLClientError>;

    /// Submits an account update with proof to the OL chain sequencer.
    async fn submit_update(&self, update: SnarkAccountUpdate) -> Result<(), OLClientError>;
}

/// Retrieves inbox messages with validation checks.
///
/// This is a checked version of [`SequencerOLClient::get_inbox_messages`] that validates:
/// - The slot range is valid (`min_slot <= max_slot`)
/// - The returned message count matches the expected number of slots
pub async fn get_inbox_messages_checked(
    client: &impl SequencerOLClient,
    min_slot: u64,
    max_slot: u64,
) -> Result<Vec<OLBlockData>, OLClientError> {
    if max_slot < min_slot {
        return Err(OLClientError::InvalidSlotRange { min_slot, max_slot });
    }

    let expected_len = (max_slot - min_slot + 1) as usize;
    let res = client.get_inbox_messages(min_slot, max_slot).await?;
    if res.len() != expected_len {
        return Err(OLClientError::UnexpectedInboxMessageCount {
            expected: expected_len,
            actual: res.len(),
        });
    }

    Ok(res)
}

/// Errors that can occur when interacting with the OL client.
#[derive(Debug, Error)]
pub enum OLClientError {
    /// End slot is less than or equal to start slot.
    #[error(
        "invalid slot range: end_slot ({max_slot}) must be greater than start_slot ({min_slot})"
    )]
    InvalidSlotRange { min_slot: u64, max_slot: u64 },

    /// Received a different number of blocks than expected.
    #[error("unexpected block count: expected {expected} blocks, got {actual}")]
    UnexpectedBlockCount { expected: usize, actual: usize },

    /// Received a different number of operation lists than expected.
    #[error("unexpected operation count: expected {expected} operation lists, got {actual}")]
    UnexpectedOperationCount { expected: usize, actual: usize },

    /// Received a different number of operation lists than expected.
    #[error("unexpected inbox message count: expected {expected} message lists, got {actual}")]
    UnexpectedInboxMessageCount { expected: usize, actual: usize },

    /// Chain status slots are not in the correct order (latest >= confirmed >= finalized).
    #[error("unexpected chain status slot order: {latest} >= {confirmed} >= {finalized}")]
    InvalidChainStatusSlotOrder {
        latest: u64,
        confirmed: u64,
        finalized: u64,
    },

    /// Network-related error occurred.
    #[error("network error: {0}")]
    Network(String),

    /// RPC call failed.
    #[error("rpc error: {0}")]
    Rpc(String),

    /// Other unspecified error.
    #[error(transparent)]
    Other(#[from] eyre::Error),
}

impl OLClientError {
    /// Creates a network error.
    pub fn network(msg: impl Into<String>) -> Self {
        Self::Network(msg.into())
    }

    /// Creates an RPC error.
    pub fn rpc(msg: impl Into<String>) -> Self {
        Self::Rpc(msg.into())
    }

    /// Returns true if the error is retryable (transient network/RPC errors).
    ///
    /// Validation errors and other non-transient errors are not retryable.
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Network(_) | Self::Rpc(_))
    }
}
