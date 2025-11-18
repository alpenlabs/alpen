//! Core types for mempool operations.

use std::collections::HashMap;

use strata_identifiers::OLTxId;
use strata_ol_chain_types_new::OLTransaction;

/// Default maximum number of transactions in mempool.
pub const DEFAULT_MAX_TX_COUNT: usize = 10_000;

/// Default maximum size of a single transaction (1 MB).
pub const DEFAULT_MAX_TX_SIZE: usize = 1024 * 1024;

/// Default maximum number of transactions per account.
pub const DEFAULT_MAX_TXS_PER_ACCOUNT: usize = 16;

/// Configuration for mempool behavior.
#[derive(Debug, Clone)]
pub struct MempoolConfig {
    /// Maximum number of transactions in mempool.
    pub max_tx_count: usize,

    /// Maximum size of a single transaction in bytes.
    pub max_tx_size: usize,

    /// Maximum number of transactions per account.
    pub max_txs_per_account: usize,
}

impl Default for MempoolConfig {
    fn default() -> Self {
        Self {
            max_tx_count: DEFAULT_MAX_TX_COUNT,
            max_tx_size: DEFAULT_MAX_TX_SIZE,
            max_txs_per_account: DEFAULT_MAX_TXS_PER_ACCOUNT,
        }
    }
}

impl MempoolConfig {
    /// Maximum total size in bytes (derived from count and individual size limits).
    pub fn max_total_size(&self) -> usize {
        self.max_tx_count * self.max_tx_size
    }
}

/// Statistics about the mempool state.
#[derive(Clone, Debug, Default)]
pub struct MempoolStats {
    /// Number of transactions currently in mempool.
    pub current_tx_count: usize,

    /// Total size in bytes of transactions currently in mempool.
    pub current_total_size: usize,

    /// Total number of transactions enqueued since startup.
    pub enqueued_tx_total: u64,

    /// Total number of transactions rejected since startup.
    pub rejected_tx_total: u64,

    /// Total number of transactions evicted since startup.
    pub evicted_tx_total: u64,

    /// Breakdown of rejections by reason.
    pub rejections_by_reason: HashMap<String, u64>,
}

/// Chain tip update for mempool lifecycle management.
///
/// Provides all information the mempool needs to maintain consistency with the chain tip.
/// This is derived from FCM's `TipUpdate` events and includes additional derived information
/// like mined transactions and orphaned transactions.
#[derive(Debug, Clone)]
pub struct ChainTipUpdate {
    /// Current slot after this update.
    pub current_slot: u64,

    /// Transaction IDs that were included in finalized blocks.
    pub mined_transactions: Vec<OLTxId>,

    /// Kind of update (commit or reorg).
    pub update_kind: PoolUpdateKind,

    /// Transactions from orphaned blocks (only present for reorgs).
    pub orphaned_txs: Vec<OLTransaction>,
}

/// Type of canonical state update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolUpdateKind {
    /// Normal chain progression - new blocks committed.
    Commit,

    /// Chain reorganization - some blocks orphaned.
    Reorg,
}
