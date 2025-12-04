//! Event types for mempool notifications.
//!
//! Provides types for subscribing to mempool state changes such as transaction additions,
//! removals, and evictions.

use strata_identifiers::OLTxId;

/// Events emitted by the mempool for state changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MempoolEvent {
    /// A transaction was successfully added to the mempool.
    TransactionAdded {
        /// Transaction ID.
        txid: OLTxId,
        /// Priority score assigned to the transaction.
        priority: u64,
    },

    /// A transaction was removed from the mempool.
    TransactionRemoved {
        /// Transaction ID.
        txid: OLTxId,
        /// Reason for removal.
        reason: RemovalReason,
    },

    /// A transaction was evicted due to capacity limits.
    TransactionEvicted {
        /// Transaction ID that was evicted.
        txid: OLTxId,
    },
}

/// Reason why a transaction was removed from the mempool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemovalReason {
    /// Transaction was included in a block.
    Included,

    /// Transaction expired (past max_slot).
    Expired,

    /// Transaction was replaced (e.g., via RBF).
    Replaced,

    /// Manual removal (e.g., via RPC).
    Manual,

    /// Removed during reorg recovery.
    Reorg,
}

impl MempoolEvent {
    /// Get the transaction ID associated with this event.
    pub fn txid(&self) -> OLTxId {
        match self {
            Self::TransactionAdded { txid, .. }
            | Self::TransactionRemoved { txid, .. }
            | Self::TransactionEvicted { txid } => *txid,
        }
    }

    /// Check if this is a transaction added event.
    pub fn is_added(&self) -> bool {
        matches!(self, Self::TransactionAdded { .. })
    }

    /// Check if this is a transaction removed event.
    pub fn is_removed(&self) -> bool {
        matches!(self, Self::TransactionRemoved { .. })
    }

    /// Check if this is a transaction evicted event.
    pub fn is_evicted(&self) -> bool {
        matches!(self, Self::TransactionEvicted { .. })
    }
}

#[cfg(test)]
mod tests {
    use strata_identifiers::Buf32;

    use super::*;

    #[test]
    fn test_event_txid() {
        let txid = OLTxId::from(Buf32::from([1u8; 32]));

        let added = MempoolEvent::TransactionAdded {
            txid,
            priority: 100,
        };
        assert_eq!(added.txid(), txid);
        assert!(added.is_added());
        assert!(!added.is_removed());
        assert!(!added.is_evicted());

        let removed = MempoolEvent::TransactionRemoved {
            txid,
            reason: RemovalReason::Included,
        };
        assert_eq!(removed.txid(), txid);
        assert!(!removed.is_added());
        assert!(removed.is_removed());
        assert!(!removed.is_evicted());

        let evicted = MempoolEvent::TransactionEvicted { txid };
        assert_eq!(evicted.txid(), txid);
        assert!(!evicted.is_added());
        assert!(!evicted.is_removed());
        assert!(evicted.is_evicted());
    }

    #[test]
    fn test_removal_reasons() {
        let reasons = [
            RemovalReason::Included,
            RemovalReason::Expired,
            RemovalReason::Replaced,
            RemovalReason::Manual,
            RemovalReason::Reorg,
        ];

        // Just verify they're all distinct
        for (i, r1) in reasons.iter().enumerate() {
            for (j, r2) in reasons.iter().enumerate() {
                if i == j {
                    assert_eq!(r1, r2);
                } else {
                    assert_ne!(r1, r2);
                }
            }
        }
    }
}
