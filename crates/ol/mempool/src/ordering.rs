//! Mempool ordering policies and ordering keys.

use std::{cmp::Ordering, fmt::Debug};

use strata_acct_types::AccountId;
use strata_identifiers::OLTxId;

use crate::types::{OLMempoolTransaction, OLMempoolTxPayload};

/// Policy trait for computing mempool transaction priorities.
///
/// Implementations define how priorities are computed from transaction data and insertion
/// metadata. Priority ordering is interpreted with the invariant that iterating in ascending order
/// yields highest-priority transactions first.
pub trait MempoolPriorityPolicy: Clone + Copy + Debug + Send + Sync + 'static {
    /// Ordering priority used by the policy.
    type Priority: Ord + Copy + Debug + Send + Sync + 'static;

    /// Compute an ordering priority for a transaction.
    ///
    /// `txid` is provided for deterministic tie-breaking when two transactions otherwise share the
    /// same priority.
    fn compute_priority(
        tx: &OLMempoolTransaction,
        timestamp_micros: u64,
        txid: OLTxId,
    ) -> Self::Priority;
}

/// FIFO priority policy.
///
/// This is the current default behavior and will continue to be used unless another policy is
/// explicitly selected.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct FifoPriority;

impl MempoolPriorityPolicy for FifoPriority {
    type Priority = MempoolOrderingKey;

    fn compute_priority(
        tx: &OLMempoolTransaction,
        timestamp_micros: u64,
        _txid: OLTxId,
    ) -> Self::Priority {
        // `txid` tie-breaking is deferred until key-level tie-break fields are added.
        MempoolOrderingKey::for_transaction(tx, timestamp_micros)
    }
}

/// Ordering key for mempool transactions.
///
/// Provides collision-free ordering with different strategies for different transaction types:
/// - **Snark transactions**: Strict per-account seq_no ordering with FIFO tiebreaking across
///   accounts
/// - **GAM transactions**: Pure FIFO ordering by `timestamp_micros`
///
/// The `timestamp_micros` is in microseconds since UNIX epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MempoolOrderingKey {
    /// Snark account update transaction.
    ///
    /// Ordered by:
    /// 1. Within same account: seq_no (strict ordering)
    /// 2. Across accounts: timestamp (FIFO)
    Snark {
        /// Target account ID.
        account_id: AccountId,
        /// Sequence number for this account (from SnarkAccountUpdate).
        seq_no: u64,
        /// Timestamp (microseconds since UNIX epoch) for cross-account FIFO ordering.
        timestamp_micros: u64,
    },

    /// Generic account message transaction.
    ///
    /// Ordered by timestamp only (pure FIFO).
    Gam {
        /// Timestamp (microseconds since UNIX epoch) for FIFO ordering.
        timestamp_micros: u64,
    },
}

impl MempoolOrderingKey {
    /// Create ordering key for a transaction with the given timestamp_micros.
    pub(crate) fn for_transaction(tx: &OLMempoolTransaction, timestamp_micros: u64) -> Self {
        match tx.payload() {
            OLMempoolTxPayload::SnarkAccountUpdate(payload) => Self::Snark {
                account_id: *payload.target(),
                seq_no: payload.base_update().operation().seq_no(),
                timestamp_micros,
            },
            OLMempoolTxPayload::GenericAccountMessage(_) => Self::Gam { timestamp_micros },
        }
    }

    /// Get the timestamp from this ordering key.
    pub fn timestamp_micros(&self) -> u64 {
        match self {
            Self::Snark {
                timestamp_micros, ..
            } => *timestamp_micros,
            Self::Gam { timestamp_micros } => *timestamp_micros,
        }
    }
}

impl PartialOrd for MempoolOrderingKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for MempoolOrderingKey {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            // Both Snark: same account? order by seq_no, else by `timestamp_micros`
            (
                Self::Snark {
                    account_id: a1,
                    seq_no: s1,
                    timestamp_micros: t1,
                },
                Self::Snark {
                    account_id: a2,
                    seq_no: s2,
                    timestamp_micros: t2,
                },
            ) => {
                if a1 == a2 {
                    s1.cmp(s2)
                } else {
                    t1.cmp(t2)
                }
            }

            // Both GAM: order by `timestamp_micros`
            (
                Self::Gam {
                    timestamp_micros: t1,
                },
                Self::Gam {
                    timestamp_micros: t2,
                },
            ) => t1.cmp(t2),

            // Mixed Snark/GAM: use `timestamp_micros` for fair interleaving
            (
                Self::Snark {
                    timestamp_micros, ..
                },
                Self::Gam {
                    timestamp_micros: t2,
                },
            )
            | (
                Self::Gam { timestamp_micros },
                Self::Snark {
                    timestamp_micros: t2,
                    ..
                },
            ) => timestamp_micros.cmp(t2),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;

    use strata_acct_types::AccountId;

    use super::*;
    use crate::{
        test_utils::{create_test_generic_tx, create_test_snark_tx},
        types::MempoolEntry,
    };

    #[test]
    fn test_gam_ordering_by_timestamp_micros() {
        let tx = create_test_generic_tx();
        let entries: Vec<_> = (1..=3)
            .map(|ts| {
                MempoolEntry::new(
                    tx.clone(),
                    MempoolOrderingKey::Gam {
                        timestamp_micros: ts,
                    },
                    100,
                )
            })
            .collect();

        for i in 0..entries.len() - 1 {
            assert_eq!(
                entries[i].ordering_key.cmp(&entries[i + 1].ordering_key),
                Ordering::Less
            );
        }
    }

    #[test]
    fn test_snark_same_account_orders_by_seq_no() {
        let account = AccountId::from([1u8; 32]);
        let tx = create_test_snark_tx();
        let timestamps = [1_000_100, 1_000_050, 1_000_025]; // Decreasing timestamps
        let entries: Vec<_> = timestamps
            .iter()
            .enumerate()
            .map(|(i, &ts)| {
                MempoolEntry::new(
                    tx.clone(),
                    MempoolOrderingKey::Snark {
                        account_id: account,
                        seq_no: i as u64 + 1,
                        timestamp_micros: ts,
                    },
                    100,
                )
            })
            .collect();

        // Same account: seq_no determines order regardless of timestamp
        for i in 0..entries.len() - 1 {
            assert_eq!(
                entries[i].ordering_key.cmp(&entries[i + 1].ordering_key),
                Ordering::Less
            );
        }
    }

    #[test]
    fn test_snark_different_accounts_orders_by_timestamp_micros() {
        let account_a = AccountId::from([1u8; 32]);
        let account_b = AccountId::from([2u8; 32]);

        // Different accounts - should order by `timestamp_micros`
        let tx_a = create_test_snark_tx();
        let tx_b = create_test_snark_tx();

        // Lower seq_no, later timestamp
        let entry_a = MempoolEntry::new(
            tx_a,
            MempoolOrderingKey::Snark {
                account_id: account_a,
                seq_no: 5,
                timestamp_micros: 2_000_100,
            },
            100,
        );
        // Higher seq_no, earlier timestamp
        let entry_b = MempoolEntry::new(
            tx_b,
            MempoolOrderingKey::Snark {
                account_id: account_b,
                seq_no: 7,
                timestamp_micros: 1_000_050,
            },
            100,
        );

        // Different accounts: `timestamp_micros` determines order
        assert_eq!(
            entry_b.ordering_key.cmp(&entry_a.ordering_key),
            Ordering::Less
        );
    }

    #[test]
    fn test_mixed_snark_gam_orders_by_timestamp_micros() {
        let account = AccountId::from([1u8; 32]);

        let tx_snark = create_test_snark_tx();
        let tx_gam = create_test_generic_tx();

        // Snark with earlier `timestamp_micros` should come first
        let entry_snark = MempoolEntry::new(
            tx_snark,
            MempoolOrderingKey::Snark {
                account_id: account,
                seq_no: 1,
                timestamp_micros: 1_000_050,
            },
            100,
        );
        let entry_gam = MempoolEntry::new(
            tx_gam,
            MempoolOrderingKey::Gam {
                timestamp_micros: 2_000_100,
            },
            100,
        );

        // Mixed: `timestamp_micros` determines order
        assert_eq!(
            entry_snark.ordering_key.cmp(&entry_gam.ordering_key),
            Ordering::Less
        );
    }

    #[test]
    fn test_complex_ordering_scenario() {
        let (acc_a, acc_b) = (AccountId::from([1u8; 32]), AccountId::from([2u8; 32]));
        let (tx_gam, tx_snark) = (create_test_generic_tx(), create_test_snark_tx());

        let mut entries = [
            MempoolEntry::new(
                tx_gam.clone(),
                MempoolOrderingKey::Gam {
                    timestamp_micros: 1_000_010,
                },
                100,
            ),
            MempoolEntry::new(
                tx_snark.clone(),
                MempoolOrderingKey::Snark {
                    account_id: acc_a,
                    seq_no: 1,
                    timestamp_micros: 1_000_020,
                },
                100,
            ),
            MempoolEntry::new(
                tx_gam,
                MempoolOrderingKey::Gam {
                    timestamp_micros: 1_000_030,
                },
                100,
            ),
            MempoolEntry::new(
                tx_snark.clone(),
                MempoolOrderingKey::Snark {
                    account_id: acc_a,
                    seq_no: 2,
                    timestamp_micros: 1_000_040,
                },
                100,
            ),
            MempoolEntry::new(
                tx_snark,
                MempoolOrderingKey::Snark {
                    account_id: acc_b,
                    seq_no: 1,
                    timestamp_micros: 1_000_050,
                },
                100,
            ),
        ];

        entries.sort_by(|a, b| a.ordering_key.cmp(&b.ordering_key));
        let timestamps: Vec<u64> = entries
            .iter()
            .map(|e| e.ordering_key.timestamp_micros())
            .collect();
        assert_eq!(
            timestamps,
            vec![1_000_010, 1_000_020, 1_000_030, 1_000_040, 1_000_050]
        );
    }
}
