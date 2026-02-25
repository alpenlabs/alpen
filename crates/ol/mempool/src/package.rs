//! Package-level mempool bookkeeping.
//!
//! A package is a group of transactions that share an intrinsic ordering rule.

use std::collections::{BTreeMap, HashMap, VecDeque};

use strata_acct_types::AccountId;
use strata_identifiers::OLTxId;
use strata_ol_chain_types_new::{OLTransaction, TransactionPayload};
use tracing::error;

use crate::ordering::MempoolPriorityPolicy;

/// Identifies a package by its grouping key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PackageKey {
    /// Package for snark account updates targeting a specific account.
    SnarkAccountUpdate(AccountId),

    /// Standalone package for a transaction with no intrinsic dependencies.
    Standalone(OLTxId),
}

/// A transaction's package identity and intra-package position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PackageMember {
    /// Per-account ordered snark update.
    SnarkAccountUpdate {
        /// Target account ID.
        account_id: AccountId,

        /// Per-account sequence number.
        seq_no: u64,
    },

    /// Standalone transaction with no intrinsic dependencies.
    Standalone {
        /// Transaction ID.
        txid: OLTxId,
    },
}

impl PackageMember {
    /// Construct package membership metadata for a transaction.
    pub(crate) fn from_tx(tx: &OLTransaction, txid: OLTxId) -> Self {
        match tx.payload() {
            TransactionPayload::SnarkAccountUpdate(payload) => Self::SnarkAccountUpdate {
                account_id: *payload.target(),
                seq_no: payload.operation().update().seq_no(),
            },
            TransactionPayload::GenericAccountMessage(_) => Self::Standalone { txid },
        }
    }

    /// Return this member's package key.
    pub(crate) fn package_key(&self) -> PackageKey {
        match self {
            Self::SnarkAccountUpdate { account_id, .. } => {
                PackageKey::SnarkAccountUpdate(*account_id)
            }
            Self::Standalone { txid } => PackageKey::Standalone(*txid),
        }
    }
}

/// Internal package-index invariant violations used for diagnostics.
#[derive(Debug, thiserror::Error)]
pub(crate) enum PackageInvariantError {
    /// Attempted to insert a txid that already exists in the package index.
    #[error("duplicate txid in package index: {0:?}")]
    DuplicateTxId(OLTxId),

    /// Transaction was mapped to a package key but package entry was missing.
    #[error("missing package entry for key: {0:?}")]
    MissingPackage(PackageKey),

    /// Package entry shape and member shape did not match.
    #[error("package/member mismatch (key: {key:?}, member: {member:?})")]
    PackageMemberMismatch {
        key: PackageKey,
        member: PackageMember,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PackageTx<Prio: MempoolPriorityPolicy> {
    txid: OLTxId,
    priority: Prio::Priority,
}

/// Package contents with explicit intrinsic ordering semantics.
#[derive(Debug, Clone)]
enum PackageContents<Prio: MempoolPriorityPolicy> {
    /// Ordered by snark seq_no.
    SnarkAccountUpdate(BTreeMap<u64, PackageTx<Prio>>),

    /// Single standalone transaction.
    Standalone(Option<PackageTx<Prio>>),
}

impl<Prio: MempoolPriorityPolicy> PackageContents<Prio> {
    fn new_for_member(member: PackageMember) -> Self {
        match member {
            PackageMember::SnarkAccountUpdate { .. } => Self::SnarkAccountUpdate(BTreeMap::new()),
            PackageMember::Standalone { .. } => Self::Standalone(None),
        }
    }

    fn front(&self) -> Option<PackageTx<Prio>> {
        match self {
            Self::SnarkAccountUpdate(txs) => txs.first_key_value().map(|(_, tx)| *tx),
            Self::Standalone(tx) => *tx,
        }
    }

    fn insert(
        &mut self,
        member: PackageMember,
        txid: OLTxId,
        priority: Prio::Priority,
    ) -> Result<(), PackageInvariantError> {
        match (self, member) {
            (Self::SnarkAccountUpdate(txs), PackageMember::SnarkAccountUpdate { seq_no, .. }) => {
                txs.insert(seq_no, PackageTx { txid, priority });
                Ok(())
            }
            (Self::Standalone(slot), PackageMember::Standalone { .. }) => {
                if slot.is_some() {
                    debug_assert!(false, "standalone package already populated for member");
                    return Err(PackageInvariantError::PackageMemberMismatch {
                        key: member.package_key(),
                        member,
                    });
                }
                *slot = Some(PackageTx { txid, priority });
                Ok(())
            }
            _ => {
                debug_assert!(
                    false,
                    "package content/member mismatch for member {member:?}"
                );
                Err(PackageInvariantError::PackageMemberMismatch {
                    key: member.package_key(),
                    member,
                })
            }
        }
    }

    fn remove(&mut self, member: PackageMember, txid: OLTxId) -> Result<(), PackageInvariantError> {
        match (self, member) {
            (Self::SnarkAccountUpdate(txs), PackageMember::SnarkAccountUpdate { seq_no, .. }) => {
                txs.remove(&seq_no);
                Ok(())
            }
            (Self::Standalone(slot), PackageMember::Standalone { .. }) => {
                if slot.as_ref().is_some_and(|tx| tx.txid == txid) {
                    *slot = None;
                    return Ok(());
                }
                debug_assert!(
                    false,
                    "standalone package missing expected tx {txid:?} for member"
                );
                Err(PackageInvariantError::PackageMemberMismatch {
                    key: member.package_key(),
                    member,
                })
            }
            _ => {
                debug_assert!(
                    false,
                    "package content/member mismatch for member {member:?}"
                );
                Err(PackageInvariantError::PackageMemberMismatch {
                    key: member.package_key(),
                    member,
                })
            }
        }
    }

    fn is_empty(&self) -> bool {
        match self {
            Self::SnarkAccountUpdate(txs) => txs.is_empty(),
            Self::Standalone(tx) => tx.is_none(),
        }
    }

    fn ordered_txs(&self) -> VecDeque<PackageTx<Prio>> {
        match self {
            Self::SnarkAccountUpdate(txs) => txs.values().copied().collect(),
            Self::Standalone(Some(tx)) => VecDeque::from([*tx]),
            Self::Standalone(None) => VecDeque::new(),
        }
    }
}

/// Single materialized package entry.
#[derive(Debug, Clone)]
struct PackageEntry<Prio: MempoolPriorityPolicy> {
    key: PackageKey,
    contents: PackageContents<Prio>,
    front_priority: Option<Prio::Priority>,
}

impl<Prio: MempoolPriorityPolicy> PackageEntry<Prio> {
    fn new(key: PackageKey, member: PackageMember) -> Self {
        Self {
            key,
            contents: PackageContents::new_for_member(member),
            front_priority: None,
        }
    }

    fn insert(
        &mut self,
        member: PackageMember,
        txid: OLTxId,
        priority: Prio::Priority,
    ) -> Result<(), PackageInvariantError> {
        if member.package_key() != self.key {
            debug_assert!(
                false,
                "package/member key mismatch on insert for key {:?}, member {:?}",
                self.key, member
            );
            return Err(PackageInvariantError::PackageMemberMismatch {
                key: self.key,
                member,
            });
        }

        self.contents.insert(member, txid, priority)?;
        self.front_priority = self.contents.front().map(|tx| tx.priority);
        Ok(())
    }

    fn remove(&mut self, member: PackageMember, txid: OLTxId) -> Result<(), PackageInvariantError> {
        if member.package_key() != self.key {
            debug_assert!(
                false,
                "package/member key mismatch on remove for key {:?}, member {:?}",
                self.key, member
            );
            return Err(PackageInvariantError::PackageMemberMismatch {
                key: self.key,
                member,
            });
        }

        self.contents.remove(member, txid)?;
        self.front_priority = self.contents.front().map(|tx| tx.priority);
        Ok(())
    }

    fn front_priority(&self) -> Option<Prio::Priority> {
        self.front_priority
    }

    fn is_empty(&self) -> bool {
        self.contents.is_empty()
    }

    fn ordered_txs(&self) -> VecDeque<PackageTx<Prio>> {
        self.contents.ordered_txs()
    }
}

/// Manages package membership and package-first candidate ordering.
#[derive(Debug, Clone)]
pub(crate) struct PackageManager<Prio: MempoolPriorityPolicy> {
    packages: HashMap<PackageKey, PackageEntry<Prio>>,
    tx_to_package: HashMap<OLTxId, PackageKey>,
}

impl<Prio: MempoolPriorityPolicy> PackageManager<Prio> {
    /// Create an empty package manager.
    pub(crate) fn new() -> Self {
        Self {
            packages: HashMap::new(),
            tx_to_package: HashMap::new(),
        }
    }

    /// Insert a tx into package bookkeeping.
    pub(crate) fn insert(&mut self, txid: OLTxId, tx: &OLTransaction, priority: Prio::Priority) {
        let member = PackageMember::from_tx(tx, txid);
        if let Err(err) = self.insert_member(txid, member, priority) {
            error!(
                ?txid,
                error = %err,
                "package bookkeeping invariant violation on insert"
            );
        }
    }

    fn insert_member(
        &mut self,
        txid: OLTxId,
        member: PackageMember,
        priority: Prio::Priority,
    ) -> Result<(), PackageInvariantError> {
        if self.tx_to_package.contains_key(&txid) {
            return Err(PackageInvariantError::DuplicateTxId(txid));
        }

        let package_key = member.package_key();
        let entry = self
            .packages
            .entry(package_key)
            .or_insert_with(|| PackageEntry::new(package_key, member));
        entry.insert(member, txid, priority)?;

        self.tx_to_package.insert(txid, package_key);
        Ok(())
    }

    /// Remove a tx from package bookkeeping.
    ///
    /// If the tx does not exist, this is a no-op.
    pub(crate) fn remove(&mut self, txid: OLTxId, tx: &OLTransaction) {
        let member = PackageMember::from_tx(tx, txid);
        if let Err(err) = self.remove_member(txid, member) {
            error!(
                ?txid,
                error = %err,
                "package bookkeeping invariant violation on remove"
            );
        }
    }

    fn remove_member(
        &mut self,
        txid: OLTxId,
        member: PackageMember,
    ) -> Result<(), PackageInvariantError> {
        let Some(&package_key) = self.tx_to_package.get(&txid) else {
            return Ok(());
        };

        if member.package_key() != package_key {
            debug_assert!(
                false,
                "tx/package-key mismatch for tx {txid:?}: mapped {package_key:?}, member {member:?}"
            );
            return Err(PackageInvariantError::PackageMemberMismatch {
                key: package_key,
                member,
            });
        }

        let Some(entry) = self.packages.get_mut(&package_key) else {
            return Err(PackageInvariantError::MissingPackage(package_key));
        };

        entry.remove(member, txid)?;
        if entry.is_empty() {
            self.packages.remove(&package_key);
        }

        self.tx_to_package.remove(&txid);
        Ok(())
    }

    /// Return candidate txids in package-first order.
    ///
    /// This computes package-front ordering on demand in `O(p log p)` per call,
    /// where `p` is number of active packages.
    pub(crate) fn iter_candidates(&self, limit: usize) -> Vec<OLTxId> {
        let mut package_queues: HashMap<PackageKey, VecDeque<PackageTx<Prio>>> = HashMap::new();
        let mut ordering_index: BTreeMap<(Prio::Priority, OLTxId), PackageKey> = BTreeMap::new();

        for (package_key, entry) in &self.packages {
            debug_assert_eq!(*package_key, entry.key);
            let queue = entry.ordered_txs();
            if let (Some(front_priority), Some(front)) = (entry.front_priority(), queue.front()) {
                ordering_index.insert((front_priority, front.txid), *package_key);
            }
            package_queues.insert(*package_key, queue);
        }

        let mut result = Vec::with_capacity(limit);
        while result.len() < limit {
            let Some((_, package_key)) = ordering_index.pop_first() else {
                break;
            };

            let Some(queue) = package_queues.get_mut(&package_key) else {
                continue;
            };
            let Some(package_tx) = queue.pop_front() else {
                continue;
            };
            result.push(package_tx.txid);

            if let Some(next_front) = queue.front() {
                ordering_index.insert((next_front.priority, next_front.txid), package_key);
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ordering::{FifoPriority, FifoPriorityKey},
        test_utils::{
            create_test_account_id, create_test_generic_tx, create_test_snark_tx_with_seq_no,
            create_test_txid_with,
        },
    };

    #[test]
    fn test_iter_candidates_orders_by_package_front_priority() {
        let mut manager = PackageManager::<FifoPriority>::new();

        let acct = create_test_account_id();
        let snark_seq0 = create_test_txid_with(1);
        let snark_seq1 = create_test_txid_with(2);
        let standalone = create_test_txid_with(3);

        manager
            .insert_member(
                snark_seq1,
                PackageMember::SnarkAccountUpdate {
                    account_id: acct,
                    seq_no: 1,
                },
                FifoPriorityKey::new(20, snark_seq1),
            )
            .unwrap();
        manager
            .insert_member(
                snark_seq0,
                PackageMember::SnarkAccountUpdate {
                    account_id: acct,
                    seq_no: 0,
                },
                FifoPriorityKey::new(11, snark_seq0),
            )
            .unwrap();
        manager
            .insert_member(
                standalone,
                PackageMember::Standalone { txid: standalone },
                FifoPriorityKey::new(10, standalone),
            )
            .unwrap();

        let ordered = manager.iter_candidates(10);
        assert_eq!(ordered, vec![standalone, snark_seq0, snark_seq1]);
    }

    #[test]
    fn test_remove_updates_package_contents() {
        let mut manager = PackageManager::<FifoPriority>::new();
        let acct = create_test_account_id();
        let txid0 = create_test_txid_with(1);
        let txid1 = create_test_txid_with(2);

        manager
            .insert_member(
                txid0,
                PackageMember::SnarkAccountUpdate {
                    account_id: acct,
                    seq_no: 0,
                },
                FifoPriorityKey::new(10, txid0),
            )
            .unwrap();
        manager
            .insert_member(
                txid1,
                PackageMember::SnarkAccountUpdate {
                    account_id: acct,
                    seq_no: 1,
                },
                FifoPriorityKey::new(11, txid1),
            )
            .unwrap();

        manager
            .remove_member(
                txid0,
                PackageMember::SnarkAccountUpdate {
                    account_id: acct,
                    seq_no: 0,
                },
            )
            .unwrap();
        assert_eq!(manager.iter_candidates(10), vec![txid1]);

        manager
            .remove_member(
                txid1,
                PackageMember::SnarkAccountUpdate {
                    account_id: acct,
                    seq_no: 1,
                },
            )
            .unwrap();
        assert!(manager.iter_candidates(10).is_empty());
    }

    #[test]
    fn test_package_key_for_snark_is_account_scoped() {
        let tx = create_test_snark_tx_with_seq_no(7, 3);
        let txid = tx.compute_txid();
        let key = PackageMember::from_tx(&tx, txid).package_key();
        assert_eq!(
            key,
            PackageKey::SnarkAccountUpdate(
                tx.target()
                    .expect("all OLTransaction payload variants must have a target")
            )
        );
    }

    #[test]
    fn test_package_key_for_gam_is_standalone() {
        let tx = create_test_generic_tx();
        let txid = tx.compute_txid();
        let key = PackageMember::from_tx(&tx, txid).package_key();
        assert_eq!(key, PackageKey::Standalone(txid));
    }

    #[test]
    fn test_package_member_snark_has_account_scoped_package_key() {
        let account = create_test_account_id();
        let member = PackageMember::SnarkAccountUpdate {
            account_id: account,
            seq_no: 10,
        };
        assert_eq!(
            member.package_key(),
            PackageKey::SnarkAccountUpdate(account)
        );
    }

    #[test]
    fn test_package_member_standalone_has_txid_scoped_package_key() {
        let txid = create_test_txid_with(44);
        let member = PackageMember::Standalone { txid };
        assert_eq!(member.package_key(), PackageKey::Standalone(txid));
    }

    #[test]
    fn test_package_member_for_snark_includes_seq_no() {
        let tx = create_test_snark_tx_with_seq_no(5, 42);
        let txid = tx.compute_txid();
        let member = PackageMember::from_tx(&tx, txid);
        match member {
            PackageMember::SnarkAccountUpdate { seq_no, .. } => assert_eq!(seq_no, 42),
            PackageMember::Standalone { .. } => panic!("expected snark member"),
        }
    }

    #[test]
    fn test_package_member_for_gam_is_standalone() {
        let tx = create_test_generic_tx();
        let txid = tx.compute_txid();
        let member = PackageMember::from_tx(&tx, txid);
        assert_eq!(member, PackageMember::Standalone { txid });
    }
}
