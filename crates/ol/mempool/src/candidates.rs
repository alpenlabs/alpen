//! Account-ordered candidate selection over a fixed mempool snapshot.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::vec::IntoIter;

use strata_acct_types::AccountId;
use strata_identifiers::OLTxId;
use strata_ol_tx_types_v1::{OLTransactionV1, TransactionPayloadV1};

/// Unix timestamp in microseconds.
type TimestampMicros = u64;

/// A transaction retained for one block assembly attempt.
#[derive(Debug, Clone)]
pub struct MempoolCandidate {
    txid: OLTxId,
    transaction: Arc<OLTransactionV1>,
    timestamp_micros: TimestampMicros,
    account_seqno: Option<(AccountId, u64)>,
}

impl MempoolCandidate {
    /// Returns the transaction ID.
    pub fn txid(&self) -> OLTxId {
        self.txid
    }

    /// Returns the transaction captured in this snapshot.
    pub fn transaction(&self) -> &OLTransactionV1 {
        &self.transaction
    }

    /// Takes the shared transaction body out of the candidate.
    pub fn into_transaction(self) -> Arc<OLTransactionV1> {
        self.transaction
    }

    /// Returns the account and sequence number for a snark update.
    pub fn account_seqno(&self) -> Option<(AccountId, u64)> {
        self.account_seqno
    }

    fn priority(&self) -> (TimestampMicros, OLTxId) {
        (self.timestamp_micros, self.txid)
    }
}

/// Selects updates in account sequence order and eligible transactions in arrival order.
///
/// Each candidate is yielded at most once. Yielding an update makes its successor
/// eligible; callers must skip the account if the update cannot proceed. The
/// snapshot shares transaction bodies with the mempool. Later arrivals, replacements,
/// and removals do not change it.
#[derive(Debug, Default)]
pub struct MempoolCandidates {
    ready: BTreeMap<(TimestampMicros, OLTxId), MempoolCandidate>,
    successors: HashMap<AccountId, IntoIter<MempoolCandidate>>,
    skipped_accounts: HashSet<AccountId>,
}

impl MempoolCandidates {
    /// Snapshots transactions and their ordering metadata without copying bodies.
    pub fn build_snapshot<'a>(
        transactions: impl IntoIterator<Item = (OLTxId, &'a Arc<OLTransactionV1>, TimestampMicros)>,
    ) -> Self {
        let mut selection = Self::default();
        let mut accounts: HashMap<AccountId, Vec<MempoolCandidate>> = HashMap::new();
        for (txid, tx, timestamp_micros) in transactions {
            let account_seqno = match tx.payload() {
                TransactionPayloadV1::SnarkAccountUpdate(payload) => {
                    Some((*payload.target(), payload.operation().update().seq_no()))
                }
                TransactionPayloadV1::GenericAccountMessage(_) => None,
            };
            let candidate = MempoolCandidate {
                txid,
                transaction: Arc::clone(tx),
                timestamp_micros,
                account_seqno,
            };
            if let Some((account, _)) = account_seqno {
                accounts.entry(account).or_default().push(candidate);
            } else {
                selection.ready.insert(candidate.priority(), candidate);
            }
        }
        for (account, mut updates) in accounts {
            updates.sort_unstable_by_key(|candidate| candidate.account_seqno);
            let mut updates = updates.into_iter();
            if let Some(first) = updates.next() {
                selection.ready.insert(first.priority(), first);
                selection.successors.insert(account, updates);
            }
        }
        selection
    }

    /// Skips this account's remaining updates for this snapshot only.
    pub fn skip_account(&mut self, account: AccountId) {
        self.skipped_accounts.insert(account);
        self.successors.remove(&account);
    }
}

impl Iterator for MempoolCandidates {
    type Item = MempoolCandidate;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some((_, candidate)) = self.ready.pop_first() {
            if let Some((account, _)) = candidate.account_seqno {
                if self.skipped_accounts.contains(&account) {
                    continue;
                }
                if let Some(next) = self.successors.get_mut(&account).and_then(Iterator::next) {
                    self.ready.insert(next.priority(), next);
                }
            }
            return Some(candidate);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{create_test_generic_tx_for_account, create_test_snark_tx_with_seq_no};

    #[test]
    fn test_arrival_order_applies_only_to_eligible_updates() {
        let successor = Arc::new(create_test_snark_tx_with_seq_no(1, 1));
        let independent = Arc::new(create_test_snark_tx_with_seq_no(2, 0));
        let message = Arc::new(create_test_generic_tx_for_account(1));
        let predecessor = Arc::new(create_test_snark_tx_with_seq_no(1, 0));
        let transactions = [
            (&successor, 10),
            (&independent, 20),
            (&message, 25),
            (&predecessor, 30),
        ];
        let candidates = MempoolCandidates::build_snapshot(
            transactions.map(|(tx, timestamp)| (tx.compute_txid(), tx, timestamp)),
        );
        assert_eq!(
            candidates
                .map(|candidate| candidate.txid())
                .collect::<Vec<_>>(),
            [&independent, &message, &predecessor, &successor].map(|tx| tx.compute_txid())
        );
    }

    #[test]
    fn test_skipping_account_preserves_independent_updates_and_messages() {
        let transactions = [
            create_test_snark_tx_with_seq_no(1, 0),
            create_test_snark_tx_with_seq_no(1, 1),
            create_test_snark_tx_with_seq_no(2, 0),
            create_test_generic_tx_for_account(1),
        ]
        .map(Arc::new);
        let mut candidates = MempoolCandidates::build_snapshot(
            transactions
                .iter()
                .enumerate()
                .map(|(index, tx)| (tx.compute_txid(), tx, index as u64)),
        );
        let first = candidates.next().unwrap();
        candidates.skip_account(first.account_seqno().unwrap().0);
        assert_eq!(
            candidates
                .map(|candidate| candidate.txid())
                .collect::<Vec<_>>(),
            transactions[2..]
                .iter()
                .map(|tx| tx.compute_txid())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_equal_timestamps_preserve_every_candidate_in_deterministic_order() {
        let transactions = [
            create_test_snark_tx_with_seq_no(1, 0),
            create_test_snark_tx_with_seq_no(1, 1),
            create_test_generic_tx_for_account(1),
            create_test_generic_tx_for_account(2),
        ]
        .map(Arc::new);
        let entries = transactions
            .iter()
            .map(|tx| (tx.compute_txid(), tx, 10))
            .collect::<Vec<_>>();
        let forward = MempoolCandidates::build_snapshot(entries.iter().copied())
            .map(|candidate| candidate.txid())
            .collect::<Vec<_>>();
        let reverse = MempoolCandidates::build_snapshot(entries.into_iter().rev())
            .map(|candidate| candidate.txid())
            .collect::<Vec<_>>();
        assert_eq!(forward, reverse);
        assert_eq!(forward.len(), transactions.len());
        assert!(
            forward
                .iter()
                .position(|id| *id == transactions[0].compute_txid())
                < forward
                    .iter()
                    .position(|id| *id == transactions[1].compute_txid())
        );
    }
}
