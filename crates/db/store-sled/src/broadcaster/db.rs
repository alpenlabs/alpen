use sled::transaction::ConflictableTransactionError;
use strata_db_types::DbResult;
use strata_db_types::common::L1TxId;
use strata_db_types::errors::DbError;
use strata_db_types::fee_bump::{TxNodeId, TxNodeRecord};
use strata_db_types::l1_broadcast::{L1BroadcastDatabase, L1TxEntry, L1TxStatus};
use strata_primitives::buf::Buf32;
use typed_sled::error::Error as TSledError;
use typed_sled::tree::SledTransactionalTree;

use super::schemas::{
    BcastActiveL1TxNodeSchema, BcastL1TxIdSchema, BcastL1TxNodeSchema, BcastL1TxSchema,
};
use crate::define_sled_database;
use crate::utils::{conv_sled_err, find_next_available_id, first, second};

define_sled_database!(
    pub struct L1BroadcastDBSled {
        tx_id_tree: BcastL1TxIdSchema,
        tx_tree: BcastL1TxSchema,
        tx_node_tree: BcastL1TxNodeSchema,
        active_tx_node_tree: BcastActiveL1TxNodeSchema,
    }
);

impl L1BroadcastDBSled {
    fn get_next_idx(&self) -> DbResult<u64> {
        match self.tx_id_tree.last().map_err(conv_sled_err)? {
            Some((idx, _)) => Ok(idx + 1),
            None => Ok(0),
        }
    }

    fn pair_indices(&self, commit_txid: Buf32, reveal_txid: Buf32) -> DbResult<Option<(u64, u64)>> {
        let (mut commit_idx, mut reveal_idx) = (None, None);
        for item in self.tx_id_tree.iter() {
            let (idx, txid) = item.map_err(conv_sled_err)?;
            if txid == commit_txid {
                commit_idx = Some(idx);
            } else if txid == reveal_txid {
                reveal_idx = Some(idx);
            }
            if commit_idx.is_some() && reveal_idx.is_some() {
                break;
            }
        }
        Ok(commit_idx.zip(reveal_idx))
    }
}

impl L1BroadcastDatabase for L1BroadcastDBSled {
    fn put_tx_entry(&self, txid: Buf32, txentry: L1TxEntry) -> DbResult<Option<u64>> {
        let next = self.get_next_idx()?;
        let idx =
            self.config
                .with_retry((&self.tx_tree, &self.tx_id_tree), |(txtree, txidtree)| {
                    let idx = if txtree.get(&txid)?.is_none() {
                        let nxt = find_next_available_id(&txidtree, next)?;
                        txidtree.insert(&nxt, &txid)?;
                        Some(nxt)
                    } else {
                        None
                    };
                    txtree.insert(&txid, &txentry)?;
                    Ok(idx)
                })?;
        Ok(idx)
    }

    fn put_tx_entry_pair(
        &self,
        commit: (Buf32, L1TxEntry),
        reveal: (Buf32, L1TxEntry),
    ) -> DbResult<Option<(u64, u64)>> {
        if commit.0 == reveal.0 {
            return Err(DbError::Other("commit and reveal txids must differ".into()));
        }
        let next = self.get_next_idx()?;
        let existing_indices = if self
            .tx_tree
            .get(&commit.0)
            .map_err(conv_sled_err)?
            .is_some()
            && self
                .tx_tree
                .get(&reveal.0)
                .map_err(conv_sled_err)?
                .is_some()
        {
            self.pair_indices(commit.0, reveal.0)?
        } else {
            None
        };
        self.config
            .with_retry((&self.tx_tree, &self.tx_id_tree), |(txs, ids)| {
                match (txs.get(&commit.0)?, txs.get(&reveal.0)?) {
                    (None, None) => {
                        let commit_idx = find_next_available_id(&ids, next)?;
                        let reveal_idx = find_next_available_id(&ids, commit_idx + 1)?;
                        ids.insert(&commit_idx, &commit.0)?;
                        ids.insert(&reveal_idx, &reveal.0)?;
                        txs.insert(&commit.0, &commit.1)?;
                        txs.insert(&reveal.0, &reveal.1)?;
                        Ok(Some((commit_idx, reveal_idx)))
                    }
                    (Some(old_commit), Some(old_reveal)) => {
                        if old_commit.status.may_be_live() || old_reveal.status.may_be_live() {
                            return if old_commit.tx_raw() == commit.1.tx_raw()
                                && old_reveal.tx_raw() == reveal.1.tx_raw()
                            {
                                Ok(None)
                            } else {
                                Err(ConflictableTransactionError::Abort(TSledError::abort(
                                    DbError::Other(
                                        "commit/reveal pair conflicts with existing entries".into(),
                                    ),
                                )))
                            };
                        }
                        let Some((commit_idx, reveal_idx)) = existing_indices else {
                            return Err(ConflictableTransactionError::Abort(TSledError::abort(
                                DbError::Other(
                                    "existing commit/reveal pair is missing broadcast indices"
                                        .into(),
                                ),
                            )));
                        };
                        if ids.get(&commit_idx)? != Some(commit.0)
                            || ids.get(&reveal_idx)? != Some(reveal.0)
                        {
                            return Err(ConflictableTransactionError::Abort(TSledError::abort(
                                DbError::Other("commit/reveal broadcast indices changed".into()),
                            )));
                        }
                        txs.insert(&commit.0, &commit.1)?;
                        txs.insert(&reveal.0, &reveal.1)?;
                        Ok(Some((commit_idx, reveal_idx)))
                    }
                    _ => Err(ConflictableTransactionError::Abort(TSledError::abort(
                        DbError::Other("commit/reveal pair conflicts with existing entries".into()),
                    ))),
                }
            })
    }

    fn put_tx_entry_by_idx(&self, idx: u64, txentry: L1TxEntry) -> DbResult<()> {
        let Some(txid) = self.tx_id_tree.get(&idx).map_err(conv_sled_err)? else {
            return Err(DbError::Other(format!(
                "Entry does not exist for idx {idx:?}"
            )));
        };

        // Read and write in one retried transaction. The fee bumper can mark an entry
        // `Replaced` concurrently, and a non-atomic read-then-write here would resurrect the
        // superseded transaction with whatever status the caller last observed.
        self.config
            .with_retry((&self.tx_tree, &self.tx_id_tree), |(txtree, _)| {
                let Some(existing) = txtree.get(&txid)? else {
                    return Err(ConflictableTransactionError::Abort(TSledError::abort(
                        DbError::Other(format!("Entry does not exist for txid at idx {idx:?}")),
                    )));
                };
                if existing.tx_raw() != txentry.tx_raw() {
                    return Err(ConflictableTransactionError::Abort(TSledError::abort(
                        DbError::Other(format!(
                            "tx entry at idx {idx:?} cannot be updated with a different transaction"
                        )),
                    )));
                }
                // `Replaced` is terminal for a txid: a concurrent fee bump has already superseded
                // it, so refuse to move the status backwards.
                if matches!(existing.status, L1TxStatus::Replaced { .. })
                    && !matches!(txentry.status, L1TxStatus::Replaced { .. })
                {
                    return Ok(());
                }
                txtree.insert(&txid, &txentry)?;
                Ok(())
            })?;

        Ok(())
    }

    fn del_tx_entry(&self, txid: Buf32) -> DbResult<bool> {
        let old_item = self.tx_tree.get(&txid).map_err(conv_sled_err)?;
        let exists = old_item.is_some();
        if exists {
            self.tx_tree
                .compare_and_swap(txid, old_item, None)
                .map_err(conv_sled_err)?;
        }
        Ok(exists)
    }

    fn del_tx_entries_from_idx(&self, start_idx: u64) -> DbResult<Vec<u64>> {
        let last_idx = self.tx_id_tree.last().map_err(conv_sled_err)?.map(first);
        let Some(last_idx) = last_idx else {
            return Ok(Vec::new());
        };

        if start_idx > last_idx {
            return Ok(Vec::new());
        }

        let deleted_indices =
            self.config
                .with_retry((&self.tx_tree, &self.tx_id_tree), |(txtree, txidtree)| {
                    let mut deleted_indices = Vec::new();
                    for idx in start_idx..=last_idx {
                        if let Some(txid) = txidtree.get(&idx)? {
                            txidtree.remove(&idx)?;
                            txtree.remove(&txid)?;
                            deleted_indices.push(idx);
                        }
                    }
                    Ok(deleted_indices)
                })?;
        Ok(deleted_indices)
    }

    fn get_tx_entry_by_id(&self, txid: Buf32) -> DbResult<Option<L1TxEntry>> {
        self.tx_tree.get(&txid).map_err(conv_sled_err)
    }

    fn get_next_tx_idx(&self) -> DbResult<u64> {
        self.get_next_idx()
    }

    fn get_txid(&self, idx: u64) -> DbResult<Option<Buf32>> {
        self.tx_id_tree.get(&idx).map_err(conv_sled_err)
    }

    fn get_tx_entry(&self, idx: u64) -> DbResult<Option<L1TxEntry>> {
        if let Some(txid) = self.get_txid(idx)? {
            self.tx_tree.get(&txid).map_err(conv_sled_err)
        } else {
            Err(DbError::Other(format!(
                "Entry does not exist for idx {idx:?}"
            )))
        }
    }

    fn get_last_tx_entry(&self) -> DbResult<Option<L1TxEntry>> {
        Ok(self.tx_tree.last().map_err(conv_sled_err)?.map(second))
    }

    fn put_replacement_tx_entry(
        &self,
        original_txid: Buf32,
        replacement_txid: Buf32,
        replacement: L1TxEntry,
    ) -> DbResult<Option<u64>> {
        let next = self.get_next_idx()?;
        self.config
            .with_retry((&self.tx_tree, &self.tx_id_tree), |(txtree, txidtree)| {
                let Some(mut original) = txtree.get(&original_txid)? else {
                    return Ok(None);
                };
                if !original.status.is_replaceable() {
                    return Ok(None);
                }

                // The swap is all-or-nothing, and `None` tells the caller nothing was written. An
                // already-present replacement row would break that contract: there would be no
                // index to report yet the original would still be transitioned. It also cannot
                // happen from a completed swap, since insert and transition commit together.
                if txtree.get(&replacement_txid)?.is_some() {
                    return Ok(None);
                }
                let idx = find_next_available_id(&txidtree, next)?;
                txidtree.insert(&idx, &replacement_txid)?;
                // The reverse link is written here so it can never disagree with the forward one.
                let mut replacement = replacement.clone();
                replacement.set_replaces(L1TxId::from(original_txid.0));
                txtree.insert(&replacement_txid, &replacement)?;

                original.status = L1TxStatus::Replaced {
                    by: L1TxId::from(replacement_txid.0),
                };
                txtree.insert(&original_txid, &original)?;

                Ok(Some(idx))
            })
    }

    fn adopt_confirmed_ancestor(
        &self,
        loser_txid: Buf32,
        winner_txid: Buf32,
        winner_status: L1TxStatus,
    ) -> DbResult<bool> {
        self.config
            .with_retry((&self.tx_tree, &self.tx_id_tree), |(txtree, _)| {
                let (Some(mut loser), Some(mut winner)) =
                    (txtree.get(&loser_txid)?, txtree.get(&winner_txid)?)
                else {
                    return Ok(false);
                };

                // A loser that has already left the bumpable states was superseded by a
                // concurrent replacement write. Reversing over it would cut that replacement
                // out of the chain while it stays indexed and broadcastable, so the chain head
                // would report the older ancestor while a live transaction spent the same
                // inputs. `put_replacement_tx_entry` and `try_mark_tx_entry_replaced` gate on
                // the same predicate.
                if !loser.status.is_replaceable() {
                    return Ok(false);
                }

                // Only reverse a link this chain actually has. Without the check a stale caller
                // could point two unrelated entries at each other.
                if !replacement_chain_reaches(&txtree, &winner.status, loser_txid)? {
                    return Ok(false);
                }

                winner.status = winner_status.clone();
                loser.status = L1TxStatus::Replaced {
                    by: L1TxId::from(winner_txid.0),
                };
                txtree.insert(&winner_txid, &winner)?;
                txtree.insert(&loser_txid, &loser)?;

                Ok(true)
            })
    }

    fn try_mark_tx_entry_replaced(&self, txid: Buf32, replacement_txid: L1TxId) -> DbResult<bool> {
        self.config
            .with_retry((&self.tx_tree, &self.tx_id_tree), |(txtree, _)| {
                let Some(mut entry) = txtree.get(&txid)? else {
                    return Ok(false);
                };
                if !entry.status.is_replaceable() {
                    return Ok(false);
                }
                entry.status = L1TxStatus::Replaced {
                    by: replacement_txid,
                };
                txtree.insert(&txid, &entry)?;
                Ok(true)
            })
    }

    fn put_tx_node(&self, node_id: TxNodeId, record: TxNodeRecord) -> DbResult<()> {
        // Record and active-set membership commit together so a crash cannot leave a live chain
        // outside the set the replacement pass scans.
        self.config.with_retry(
            (&self.tx_node_tree, &self.active_tx_node_tree),
            |(node_tree, active_tree)| {
                node_tree.insert(&node_id, &record)?;
                if record.terminal_error.is_some() {
                    active_tree.remove(&node_id)?;
                } else {
                    active_tree.insert(&node_id, &())?;
                }
                Ok(())
            },
        )?;
        Ok(())
    }

    fn get_tx_node(&self, node_id: TxNodeId) -> DbResult<Option<TxNodeRecord>> {
        self.tx_node_tree.get(&node_id).map_err(conv_sled_err)
    }

    fn get_all_tx_nodes(&self) -> DbResult<Vec<TxNodeRecord>> {
        let mut records = Vec::new();
        for item in self.tx_node_tree.iter() {
            let (_, record) = item.map_err(conv_sled_err)?;
            records.push(record);
        }
        Ok(records)
    }

    fn get_active_tx_nodes(&self) -> DbResult<Vec<TxNodeRecord>> {
        let mut records = Vec::new();
        for item in self.active_tx_node_tree.iter() {
            let (node_id, ()) = item.map_err(conv_sled_err)?;
            if let Some(record) = self.tx_node_tree.get(&node_id).map_err(conv_sled_err)? {
                records.push(record);
            }
        }
        Ok(records)
    }

    fn retire_tx_node(&self, node_id: TxNodeId, expected_active_txid: L1TxId) -> DbResult<bool> {
        self.config.with_retry(
            (&self.tx_node_tree, &self.active_tx_node_tree),
            |(node_tree, active_tree)| {
                let Some(mut record) = node_tree.get(&node_id)? else {
                    // A membership entry without a record indexes nothing; drop it.
                    active_tree.remove(&node_id)?;
                    return Ok(false);
                };
                if record.active_txid != expected_active_txid {
                    return Ok(false);
                }
                // The record is kept forever for point lookups, but a retired chain never
                // rebroadcasts or re-signs, so its raw transaction bytes are dead weight.
                // Dropping them here bounds the permanent record to metadata size; without this,
                // every finalized chain retains its active attempt's full serialized transaction.
                record.forget_all_raw_txs();
                node_tree.insert(&node_id, &record)?;
                active_tree.remove(&node_id)?;
                Ok(true)
            },
        )
    }
}

/// Bound on how far the adoption check walks a replacement chain.
///
/// Comfortably above the hop budget the broadcaster's ancestor search uses, so the check never
/// refuses a pair that search was able to find.
const MAX_ADOPTION_CHAIN_HOPS: usize = 64;

/// Reports whether following `status`'s forward [`L1TxStatus::Replaced`] links arrives at `target`.
///
/// The winner of an adoption need not be the loser's immediate parent. A chain bumped more than
/// once has intermediate attempts between them, and the miner is free to include any of them, so
/// requiring a direct link would refuse every adoption in a chain longer than two and leave the
/// caller marking a live payload invalid.
fn replacement_chain_reaches(
    txtree: &SledTransactionalTree<BcastL1TxSchema>,
    status: &L1TxStatus,
    target: Buf32,
) -> Result<bool, TSledError> {
    let L1TxStatus::Replaced { by } = status else {
        return Ok(false);
    };
    let mut current = Buf32(by.0);

    for _ in 0..MAX_ADOPTION_CHAIN_HOPS {
        if current == target {
            return Ok(true);
        }
        let Some(entry) = txtree.get(&current)? else {
            return Ok(false);
        };
        let L1TxStatus::Replaced { by } = entry.status else {
            return Ok(false);
        };
        current = Buf32(by.0);
    }

    Ok(false)
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {
    use strata_db_tests::l1_broadcast_db_tests;

    use super::*;
    use crate::sled_db_test_setup;

    sled_db_test_setup!(L1BroadcastDBSled, l1_broadcast_db_tests);
}
