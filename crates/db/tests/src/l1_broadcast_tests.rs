use bitcoin::consensus::{deserialize, serialize};
use bitcoin::hashes::Hash;
use bitcoin::{Amount, FeeRate, Transaction};
use strata_db_types::common::{L1TxId, L1WtxId};
use strata_db_types::fee_bump::{
    TerminalError, TxAttempt, TxAttemptParts, TxNodeId, TxNodeKind, TxNodeRecord,
};
use strata_db_types::l1_broadcast::{L1BroadcastDatabase, L1TxEntry, L1TxRbfInfo, L1TxStatus};
use strata_primitives::buf::Buf32;

/// Builds an unpublished entry for a transaction.
///
/// `L1TxEntry` stores opaque bytes, so the Bitcoin encoding happens here rather than in the
/// database crate.
fn tx_entry(tx: &Transaction) -> L1TxEntry {
    L1TxEntry::new_unpublished(serialize(tx))
}

/// Builds the persistent attempt material for `tx`, mirroring `strata_btcio::tx_attempt`.
fn attempt_parts(tx: &Transaction, fee_rate: FeeRate, fee: Amount) -> TxAttemptParts {
    TxAttemptParts {
        raw_tx: serialize(tx),
        txid: L1TxId::from(tx.compute_txid().to_byte_array()),
        wtxid: L1WtxId::from(tx.compute_wtxid().to_byte_array()),
        fee_rate_sat_vb: fee_rate.to_sat_per_vb_ceil(),
        fee_sats: fee.to_sat(),
    }
}

/// Builds a writer-owned unpublished entry carrying the RBF metadata for `fee_rate` and `fee`.
fn tx_entry_with_fee(tx: &Transaction, fee_rate: FeeRate, fee: Amount) -> L1TxEntry {
    L1TxEntry::from_raw_parts(
        serialize(tx),
        L1TxStatus::Unpublished,
        Some(L1TxRbfInfo {
            fee_rate_sat_vb: fee_rate.to_sat_per_vb_ceil(),
            fee_sats: fee.to_sat(),
            replaces: None,
        }),
    )
}

pub fn test_get_last_tx_entry(db: &impl L1BroadcastDatabase) {
    for _ in 0..2 {
        let (txid, txentry) = generate_l1_tx_entry();

        let _ = db.put_tx_entry(txid, txentry.clone()).unwrap();
        let last_entry = db.get_last_tx_entry().unwrap();

        assert_eq!(last_entry, Some(txentry));
    }
}

pub fn test_add_tx_new_entry(db: &impl L1BroadcastDatabase) {
    let (txid, txentry) = generate_l1_tx_entry();

    let idx = db.put_tx_entry(txid, txentry.clone()).unwrap();

    assert_eq!(idx, Some(0));

    let stored_entry = db.get_tx_entry(idx.unwrap()).unwrap();
    assert_eq!(stored_entry, Some(txentry));
}

pub fn test_put_tx_existing_entry(db: &impl L1BroadcastDatabase) {
    let (txid, mut txentry) = generate_l1_tx_entry();

    let idx = db.put_tx_entry(txid, txentry.clone()).unwrap().unwrap();

    // Update the same txid
    txentry.status = L1TxStatus::Published;
    let result = db.put_tx_entry(txid, txentry.clone());

    assert_eq!(result.unwrap(), None);
    assert_eq!(db.get_next_tx_idx().unwrap(), idx + 1);
    assert_eq!(db.get_tx_entry(idx).unwrap(), Some(txentry));
}

pub fn test_put_tx_entry_pair(db: &impl L1BroadcastDatabase) {
    let txs = get_test_bitcoin_txs();
    let pair = |tx: &Transaction| {
        (
            tx.compute_txid().as_raw_hash().to_byte_array().into(),
            L1TxEntry::from_tx(tx),
        )
    };
    let (commit_idx, reveal_idx) = db.put_tx_entry_pair(pair(&txs[0]), pair(&txs[1])).unwrap();

    assert_eq!((commit_idx, reveal_idx), (0, 1));
    assert_eq!(
        db.put_tx_entry_pair(pair(&txs[0]), pair(&txs[1])).unwrap(),
        (commit_idx, reveal_idx)
    );
    assert!(db.put_tx_entry_pair(pair(&txs[0]), pair(&txs[2])).is_err());
    for idx in [commit_idx, reveal_idx] {
        let mut entry = db.get_tx_entry(idx).unwrap().unwrap();
        entry.status = L1TxStatus::Abandoned;
        db.put_tx_entry_by_idx(idx, entry).unwrap();
    }
    assert_eq!(
        db.put_tx_entry_pair(pair(&txs[0]), pair(&txs[1])).unwrap(),
        (commit_idx, reveal_idx)
    );
    assert!([commit_idx, reveal_idx].into_iter().all(|idx| db
        .get_tx_entry(idx)
        .unwrap()
        .unwrap()
        .status
        == L1TxStatus::Unpublished));
    assert_eq!(db.get_next_tx_idx().unwrap(), 2);
}

pub fn test_update_tx_entry(db: &impl L1BroadcastDatabase) {
    let (txid, txentry) = generate_l1_tx_entry();

    // Attempt to update non-existing index
    let result = db.put_tx_entry_by_idx(0, txentry.clone());
    assert!(result.is_err());

    // Add and then update the entry by index
    let idx = db.put_tx_entry(txid, txentry.clone()).unwrap();

    let mut updated_txentry = txentry;
    updated_txentry.status = L1TxStatus::Finalized {
        confirmations: 1,
        block_hash: Buf32::zero(),
        block_height: 100,
    };

    db.put_tx_entry_by_idx(idx.unwrap(), updated_txentry.clone())
        .unwrap();

    let stored_entry = db.get_tx_entry(idx.unwrap()).unwrap();
    assert_eq!(stored_entry, Some(updated_txentry));
}

pub fn test_update_tx_entry_rejects_mismatched_tx(db: &impl L1BroadcastDatabase) {
    let (txid, txentry) = generate_l1_tx_entry();
    let txns = get_test_bitcoin_txs();
    let other_txentry = tx_entry(&txns[1]);

    let idx = db.put_tx_entry(txid, txentry.clone()).unwrap().unwrap();
    let result = db.put_tx_entry_by_idx(idx, other_txentry);

    assert!(result.is_err());
    assert_eq!(db.get_tx_entry(idx).unwrap(), Some(txentry));
}

pub fn test_get_txentry_by_idx(db: &impl L1BroadcastDatabase) {
    // Test non-existing entry
    let result = db.get_tx_entry(0);
    assert!(result.is_err());

    let (txid, txentry) = generate_l1_tx_entry();

    let idx = db.put_tx_entry(txid, txentry.clone()).unwrap();

    let stored_entry = db.get_tx_entry(idx.unwrap()).unwrap();
    assert_eq!(stored_entry, Some(txentry));
}

pub fn test_get_next_txidx(db: &impl L1BroadcastDatabase) {
    let next_txidx = db.get_next_tx_idx().unwrap();
    assert_eq!(next_txidx, 0, "The next txidx is 0 in the beginning");

    let (txid, txentry) = generate_l1_tx_entry();

    let idx = db.put_tx_entry(txid, txentry.clone()).unwrap();

    let next_txidx = db.get_next_tx_idx().unwrap();

    assert_eq!(next_txidx, idx.unwrap() + 1);
}

pub fn test_del_tx_entry_single(db: &impl L1BroadcastDatabase) {
    let (txid, txentry) = generate_l1_tx_entry();

    // Insert tx entry
    db.put_tx_entry(txid, txentry.clone())
        .expect("test: insert");

    // Verify it exists
    assert!(db.get_tx_entry_by_id(txid).expect("test: get").is_some());

    // Delete it
    let deleted = db.del_tx_entry(txid).expect("test: delete");
    assert!(
        deleted,
        "Should return true when deleting existing tx entry"
    );

    // Verify it's gone
    assert!(db
        .get_tx_entry_by_id(txid)
        .expect("test: get after delete")
        .is_none());

    // Delete again should return false
    let deleted_again = db.del_tx_entry(txid).expect("test: delete again");
    assert!(
        !deleted_again,
        "Should return false when deleting non-existent tx entry"
    );
}

pub fn test_del_tx_entries_from_idx(db: &impl L1BroadcastDatabase) {
    let txs = get_test_bitcoin_txs();

    // Generate different tx entries
    let txid1: Buf32 = txs[0].compute_txid().as_raw_hash().to_byte_array().into();
    let txid2: Buf32 = txs[1].compute_txid().as_raw_hash().to_byte_array().into();
    let txid3: Buf32 = txs[2].compute_txid().as_raw_hash().to_byte_array().into();
    let txid4: Buf32 = txs[3].compute_txid().as_raw_hash().to_byte_array().into();

    let txentry1 = tx_entry(&txs[0]);
    let txentry2 = tx_entry(&txs[1]);
    let txentry3 = tx_entry(&txs[2]);
    let txentry4 = tx_entry(&txs[3]);

    // Insert tx entries - they will get consecutive indices
    db.put_tx_entry(txid1, txentry1).expect("test: insert 1");
    db.put_tx_entry(txid2, txentry2).expect("test: insert 2");
    db.put_tx_entry(txid3, txentry3).expect("test: insert 3");
    db.put_tx_entry(txid4, txentry4).expect("test: insert 4");

    // Verify all exist by getting tx by idx
    assert!(db.get_tx_entry(0).expect("test: get idx 0").is_some());
    assert!(db.get_tx_entry(1).expect("test: get idx 1").is_some());
    assert!(db.get_tx_entry(2).expect("test: get idx 2").is_some());
    assert!(db.get_tx_entry(3).expect("test: get idx 3").is_some());

    // Delete from index 2 onwards
    let deleted_indices = db
        .del_tx_entries_from_idx(2)
        .expect("test: delete from idx 2");
    assert_eq!(deleted_indices, vec![2, 3], "Should delete indices 2 and 3");

    // Verify indices 0 and 1 still exist, indices 2 and 3 are gone
    assert!(db.get_tx_entry(0).expect("test: get idx 0 after").is_some());
    assert!(db.get_tx_entry(1).expect("test: get idx 1 after").is_some());
    assert!(
        db.get_tx_entry(2).is_err(),
        "Should error when getting deleted index 2"
    );
    assert!(
        db.get_tx_entry(3).is_err(),
        "Should error when getting deleted index 3"
    );

    // Also verify the tx entries themselves are gone
    assert!(db
        .get_tx_entry_by_id(txid3)
        .expect("test: get id 3")
        .is_none());
    assert!(db
        .get_tx_entry_by_id(txid4)
        .expect("test: get id 4")
        .is_none());
}

pub fn test_del_tx_entries_empty_database(db: &impl L1BroadcastDatabase) {
    // Delete from empty database should return empty vec
    let deleted_indices = db
        .del_tx_entries_from_idx(0)
        .expect("test: delete from empty");
    assert!(
        deleted_indices.is_empty(),
        "Should return empty vec for empty database"
    );
}

/// `Replaced` is terminal for a txid: the broadcaster must not be able to write a stale
/// pre-replacement status back over a fee bump's transition.
pub fn test_put_tx_entry_by_idx_refuses_to_unreplace(db: &impl L1BroadcastDatabase) {
    let (txid, txentry) = generate_l1_tx_entry();
    let idx = db.put_tx_entry(txid, txentry.clone()).unwrap().unwrap();

    let mut replaced = txentry.clone();
    replaced.status = L1TxStatus::Replaced {
        by: L1TxId::from([9u8; 32]),
    };
    db.put_tx_entry_by_idx(idx, replaced).unwrap();

    // A stale writer tries to move it back to Published.
    let mut stale = txentry;
    stale.status = L1TxStatus::Published;
    db.put_tx_entry_by_idx(idx, stale).unwrap();

    assert!(matches!(
        db.get_tx_entry(idx).unwrap().unwrap().status,
        L1TxStatus::Replaced { .. }
    ));
}

/// A later replacement in the same chain must still be recordable.
pub fn test_put_tx_entry_by_idx_allows_rereplacement(db: &impl L1BroadcastDatabase) {
    let (txid, txentry) = generate_l1_tx_entry();
    let idx = db.put_tx_entry(txid, txentry.clone()).unwrap().unwrap();

    let mut replaced = txentry.clone();
    replaced.status = L1TxStatus::Replaced {
        by: L1TxId::from([9u8; 32]),
    };
    db.put_tx_entry_by_idx(idx, replaced).unwrap();

    let mut replaced_again = txentry;
    replaced_again.status = L1TxStatus::Replaced {
        by: L1TxId::from([10u8; 32]),
    };
    db.put_tx_entry_by_idx(idx, replaced_again).unwrap();

    assert_eq!(
        db.get_tx_entry(idx).unwrap().unwrap().status,
        L1TxStatus::Replaced {
            by: L1TxId::from([10u8; 32])
        }
    );
}

/// The swap inserts the replacement and supersedes the original in one step.
pub fn test_put_replacement_tx_entry_swaps_atomically(db: &impl L1BroadcastDatabase) {
    let txns = get_test_bitcoin_txs();
    let original_txid: Buf32 = txns[0].compute_txid().as_raw_hash().to_byte_array().into();
    let replacement_txid: Buf32 = txns[1].compute_txid().as_raw_hash().to_byte_array().into();
    let mut original = tx_entry(&txns[0]);
    original.status = L1TxStatus::Published;
    db.put_tx_entry(original_txid, original).unwrap();

    let replacement = tx_entry(&txns[1]);
    let idx = db
        .put_replacement_tx_entry(original_txid, replacement_txid, replacement.clone())
        .unwrap()
        .expect("swap applies to a published original");

    assert_eq!(db.get_tx_entry(idx).unwrap(), Some(replacement));
    assert_eq!(
        db.get_tx_entry_by_id(original_txid)
            .unwrap()
            .unwrap()
            .status,
        L1TxStatus::Replaced {
            by: L1TxId::from(replacement_txid.0)
        }
    );
}

/// The reverse link is what lets the broadcaster walk back to the ancestors of a replacement, so
/// the swap has to record it alongside the forward one.
pub fn test_put_replacement_tx_entry_records_the_reverse_link(db: &impl L1BroadcastDatabase) {
    let txns = get_test_bitcoin_txs();
    let original_txid: Buf32 = txns[0].compute_txid().as_raw_hash().to_byte_array().into();
    let replacement_txid: Buf32 = txns[1].compute_txid().as_raw_hash().to_byte_array().into();
    let mut original = tx_entry_with_fee(
        &txns[0],
        FeeRate::from_sat_per_vb(2).unwrap(),
        Amount::from_sat(200),
    );
    original.status = L1TxStatus::Published;
    db.put_tx_entry(original_txid, original).unwrap();

    db.put_replacement_tx_entry(
        original_txid,
        replacement_txid,
        tx_entry_with_fee(
            &txns[1],
            FeeRate::from_sat_per_vb(4).unwrap(),
            Amount::from_sat(400),
        ),
    )
    .unwrap()
    .expect("swap applies to a published original");

    assert_eq!(
        db.get_tx_entry_by_id(replacement_txid)
            .unwrap()
            .unwrap()
            .rbf
            .unwrap()
            .replaces,
        Some(L1TxId::from(original_txid.0))
    );
}

/// A miner can include an original after the local node accepted its replacement. The chain then
/// has to be repointed at the winner, or every consumer walks past it and concludes the chain died.
pub fn test_adopt_confirmed_ancestor_reverses_the_chain(db: &impl L1BroadcastDatabase) {
    let txns = get_test_bitcoin_txs();
    let winner_txid: Buf32 = txns[0].compute_txid().as_raw_hash().to_byte_array().into();
    let loser_txid: Buf32 = txns[1].compute_txid().as_raw_hash().to_byte_array().into();
    let mut winner = tx_entry(&txns[0]);
    winner.status = L1TxStatus::Published;
    db.put_tx_entry(winner_txid, winner).unwrap();
    db.put_replacement_tx_entry(winner_txid, loser_txid, tx_entry(&txns[1]))
        .unwrap()
        .expect("swap applies");

    let confirmed = L1TxStatus::Confirmed {
        confirmations: 3,
        block_hash: Buf32::zero(),
        block_height: 400,
    };
    assert!(db
        .adopt_confirmed_ancestor(loser_txid, winner_txid, confirmed.clone())
        .unwrap());

    assert_eq!(
        db.get_tx_entry_by_id(winner_txid).unwrap().unwrap().status,
        confirmed
    );
    assert_eq!(
        db.get_tx_entry_by_id(loser_txid).unwrap().unwrap().status,
        L1TxStatus::Replaced {
            by: L1TxId::from(winner_txid.0)
        }
    );
}

/// The winner does not have to be the loser's immediate parent. A chain bumped twice has an
/// intermediate attempt between them, and a miner can still include the original.
pub fn test_adopt_confirmed_ancestor_reverses_a_multi_hop_chain(db: &impl L1BroadcastDatabase) {
    let txns = get_test_bitcoin_txs();
    let winner_txid: Buf32 = txns[0].compute_txid().as_raw_hash().to_byte_array().into();
    let middle_txid: Buf32 = txns[1].compute_txid().as_raw_hash().to_byte_array().into();
    let loser_txid: Buf32 = txns[2].compute_txid().as_raw_hash().to_byte_array().into();
    let mut winner = tx_entry(&txns[0]);
    winner.status = L1TxStatus::Published;
    db.put_tx_entry(winner_txid, winner).unwrap();
    db.put_replacement_tx_entry(winner_txid, middle_txid, tx_entry(&txns[1]))
        .unwrap()
        .expect("first swap applies");
    // The replacement lands `Unpublished`; only a published entry can itself be replaced.
    let mut middle = db.get_tx_entry_by_id(middle_txid).unwrap().unwrap();
    middle.status = L1TxStatus::Published;
    db.put_tx_entry(middle_txid, middle).unwrap();
    db.put_replacement_tx_entry(middle_txid, loser_txid, tx_entry(&txns[2]))
        .unwrap()
        .expect("second swap applies");

    let confirmed = L1TxStatus::Confirmed {
        confirmations: 3,
        block_hash: Buf32::zero(),
        block_height: 400,
    };
    assert!(db
        .adopt_confirmed_ancestor(loser_txid, winner_txid, confirmed.clone())
        .unwrap());

    assert_eq!(
        db.get_tx_entry_by_id(winner_txid).unwrap().unwrap().status,
        confirmed
    );
    assert_eq!(
        db.get_tx_entry_by_id(loser_txid).unwrap().unwrap().status,
        L1TxStatus::Replaced {
            by: L1TxId::from(winner_txid.0)
        }
    );
    // The intermediate keeps its forward link, which now resolves to the winner through the
    // reversed one rather than dead-ending on the loser.
    assert_eq!(
        db.get_tx_entry_by_id(middle_txid).unwrap().unwrap().status,
        L1TxStatus::Replaced {
            by: L1TxId::from(loser_txid.0)
        }
    );
}

/// A concurrent fee bump can supersede the loser while an adoption is still deciding, since the
/// adoption makes an RPC round-trip per ancestor before it writes. Reversing over the loser then
/// cuts the newer replacement out of the chain while it stays indexed and broadcastable, so the
/// chain head names the old ancestor while a live transaction spends the same inputs.
pub fn test_adopt_confirmed_ancestor_refuses_a_superseded_loser(db: &impl L1BroadcastDatabase) {
    let txns = get_test_bitcoin_txs();
    let winner_txid: Buf32 = txns[0].compute_txid().as_raw_hash().to_byte_array().into();
    let loser_txid: Buf32 = txns[1].compute_txid().as_raw_hash().to_byte_array().into();
    let newer_txid: Buf32 = txns[2].compute_txid().as_raw_hash().to_byte_array().into();
    let mut winner = tx_entry(&txns[0]);
    winner.status = L1TxStatus::Published;
    db.put_tx_entry(winner_txid, winner).unwrap();
    db.put_replacement_tx_entry(winner_txid, loser_txid, tx_entry(&txns[1]))
        .unwrap()
        .expect("first swap applies");

    // The writer's fee-bump pass supersedes the loser while the adoption is deciding.
    db.put_replacement_tx_entry(loser_txid, newer_txid, tx_entry(&txns[2]))
        .unwrap()
        .expect("second swap applies");

    assert!(!db
        .adopt_confirmed_ancestor(
            loser_txid,
            winner_txid,
            L1TxStatus::Confirmed {
                confirmations: 3,
                block_hash: Buf32::zero(),
                block_height: 400,
            },
        )
        .unwrap());

    assert_eq!(
        db.get_tx_entry_by_id(loser_txid).unwrap().unwrap().status,
        L1TxStatus::Replaced {
            by: L1TxId::from(newer_txid.0)
        },
        "the loser must keep pointing at the replacement that superseded it"
    );
    assert_eq!(
        db.get_tx_entry_by_id(winner_txid).unwrap().unwrap().status,
        L1TxStatus::Replaced {
            by: L1TxId::from(loser_txid.0)
        },
        "a refused adoption must not advance the winner either"
    );
}

/// Only a link the chain actually has may be reversed, otherwise a stale caller could point two
/// unrelated entries at each other and strand both.
pub fn test_adopt_confirmed_ancestor_refuses_an_unlinked_pair(db: &impl L1BroadcastDatabase) {
    let txns = get_test_bitcoin_txs();
    let winner_txid: Buf32 = txns[0].compute_txid().as_raw_hash().to_byte_array().into();
    let loser_txid: Buf32 = txns[1].compute_txid().as_raw_hash().to_byte_array().into();
    let mut winner = tx_entry(&txns[0]);
    winner.status = L1TxStatus::Published;
    db.put_tx_entry(winner_txid, winner).unwrap();
    db.put_tx_entry(loser_txid, tx_entry(&txns[1])).unwrap();

    assert!(!db
        .adopt_confirmed_ancestor(
            loser_txid,
            winner_txid,
            L1TxStatus::Confirmed {
                confirmations: 3,
                block_hash: Buf32::zero(),
                block_height: 400,
            },
        )
        .unwrap());
    assert_eq!(
        db.get_tx_entry_by_id(winner_txid).unwrap().unwrap().status,
        L1TxStatus::Published
    );
    assert_eq!(
        db.get_tx_entry_by_id(loser_txid).unwrap().unwrap().status,
        L1TxStatus::Unpublished
    );
}

/// If the original already confirmed the swap writes nothing at all, replacement row included.
pub fn test_put_replacement_tx_entry_writes_nothing_when_refused(db: &impl L1BroadcastDatabase) {
    let txns = get_test_bitcoin_txs();
    let original_txid: Buf32 = txns[0].compute_txid().as_raw_hash().to_byte_array().into();
    let replacement_txid: Buf32 = txns[1].compute_txid().as_raw_hash().to_byte_array().into();
    let confirmed_status = L1TxStatus::Confirmed {
        confirmations: 1,
        block_hash: Buf32::zero(),
        block_height: 100,
    };
    let mut original = tx_entry(&txns[0]);
    original.status = confirmed_status.clone();
    db.put_tx_entry(original_txid, original).unwrap();

    assert_eq!(
        db.put_replacement_tx_entry(original_txid, replacement_txid, tx_entry(&txns[1]))
            .unwrap(),
        None
    );
    assert_eq!(
        db.get_tx_entry_by_id(original_txid)
            .unwrap()
            .unwrap()
            .status,
        confirmed_status
    );
    assert_eq!(db.get_tx_entry_by_id(replacement_txid).unwrap(), None);
}

/// An already-present replacement row means an earlier swap ran. Writing again would transition
/// the original with no index to report, so the swap must refuse and leave both rows alone.
pub fn test_put_replacement_tx_entry_refuses_existing_replacement(db: &impl L1BroadcastDatabase) {
    let txns = get_test_bitcoin_txs();
    let original_txid: Buf32 = txns[0].compute_txid().as_raw_hash().to_byte_array().into();
    let replacement_txid: Buf32 = txns[1].compute_txid().as_raw_hash().to_byte_array().into();

    let mut original = tx_entry(&txns[0]);
    original.status = L1TxStatus::Published;
    db.put_tx_entry(original_txid, original.clone()).unwrap();
    db.put_tx_entry(replacement_txid, tx_entry(&txns[1]))
        .unwrap();

    assert_eq!(
        db.put_replacement_tx_entry(original_txid, replacement_txid, tx_entry(&txns[1]))
            .unwrap(),
        None
    );
    assert_eq!(
        db.get_tx_entry_by_id(original_txid)
            .unwrap()
            .unwrap()
            .status,
        L1TxStatus::Published,
        "the original must not be transitioned when the swap refuses"
    );
}

/// A published transaction can be superseded, and the transition reports that it applied.
pub fn test_try_mark_tx_entry_replaced_applies_to_published(db: &impl L1BroadcastDatabase) {
    let (txid, mut txentry) = generate_l1_tx_entry();
    txentry.status = L1TxStatus::Published;
    db.put_tx_entry(txid, txentry).unwrap();

    let replacement = L1TxId::from([9u8; 32]);
    assert!(db.try_mark_tx_entry_replaced(txid, replacement).unwrap());
    assert_eq!(
        db.get_tx_entry_by_id(txid).unwrap().unwrap().status,
        L1TxStatus::Replaced { by: replacement }
    );
}

/// A transaction that already confirmed has won. The transition must not apply, and — critically —
/// must not report success, or callers would advance metadata onto an unconfirmable replacement.
pub fn test_try_mark_tx_entry_replaced_refuses_confirmed(db: &impl L1BroadcastDatabase) {
    let (txid, mut txentry) = generate_l1_tx_entry();
    let confirmed_status = L1TxStatus::Confirmed {
        confirmations: 1,
        block_hash: Buf32::zero(),
        block_height: 100,
    };
    txentry.status = confirmed_status.clone();
    db.put_tx_entry(txid, txentry).unwrap();

    assert!(!db
        .try_mark_tx_entry_replaced(txid, L1TxId::from([9u8; 32]))
        .unwrap());
    assert_eq!(
        db.get_tx_entry_by_id(txid).unwrap().unwrap().status,
        confirmed_status
    );
}

/// An unknown txid reports no transition rather than erroring.
pub fn test_try_mark_tx_entry_replaced_missing_entry(db: &impl L1BroadcastDatabase) {
    assert!(!db
        .try_mark_tx_entry_replaced(Buf32::from([3u8; 32]), L1TxId::from([9u8; 32]))
        .unwrap());
}

pub fn test_tx_node_roundtrip(db: &impl L1BroadcastDatabase) {
    let kind = TxNodeKind::ChunkedEnvelopeReveal {
        envelope_idx: 4,
        reveal_idx: 2,
    };
    let node_id = TxNodeId::from_kind(&kind);

    assert_eq!(db.get_tx_node(node_id).unwrap(), None);

    let record = generate_tx_node_record(kind);
    db.put_tx_node(node_id, record.clone()).unwrap();

    assert_eq!(db.get_tx_node(node_id).unwrap(), Some(record));
}

pub fn test_tx_node_overwrite_and_list(db: &impl L1BroadcastDatabase) {
    assert!(db.get_all_tx_nodes().unwrap().is_empty());

    let commit_kind = TxNodeKind::ChunkedEnvelopeCommit { envelope_idx: 1 };
    let reveal_kind = TxNodeKind::ChunkedEnvelopeReveal {
        envelope_idx: 1,
        reveal_idx: 0,
    };
    let commit = generate_tx_node_record(commit_kind);
    let reveal = generate_tx_node_record(reveal_kind);

    db.put_tx_node(commit.node_id, commit.clone()).unwrap();
    db.put_tx_node(reveal.node_id, reveal.clone()).unwrap();
    assert_eq!(db.get_all_tx_nodes().unwrap().len(), 2);

    // Re-putting the same node id replaces the record rather than adding a second one.
    let txns = get_test_bitcoin_txs();
    let mut bumped = commit.clone();
    bumped.append_replacement(TxAttempt::active(
        attempt_parts(
            &txns[1],
            FeeRate::from_sat_per_vb(4).expect("test: valid fee rate"),
            Amount::from_sat(800),
        ),
        bumped.next_attempt_no(),
    ));
    db.put_tx_node(bumped.node_id, bumped.clone()).unwrap();

    assert_eq!(db.get_all_tx_nodes().unwrap().len(), 2);
    assert_eq!(db.get_tx_node(commit.node_id).unwrap(), Some(bumped));
}

pub fn test_active_tx_node_index_lifecycle(db: &impl L1BroadcastDatabase) {
    let txns = get_test_bitcoin_txs();
    let kind = TxNodeKind::SingleEnvelopeReveal { payload_idx: 11 };
    let mut record = generate_tx_node_record(kind);
    let node_id = record.node_id;

    // A non-terminal write enters the active set.
    db.put_tx_node(node_id, record.clone()).unwrap();
    assert_eq!(db.get_active_tx_nodes().unwrap(), vec![record.clone()]);

    // A terminal write leaves it, while the record stays readable for point lookups.
    record.set_terminal_error(TerminalError::MaxAttemptsReached);
    db.put_tx_node(node_id, record.clone()).unwrap();
    assert!(db.get_active_tx_nodes().unwrap().is_empty());
    assert_eq!(db.get_tx_node(node_id).unwrap(), Some(record.clone()));

    // A writer rebuild clears the terminal error and re-enters the set.
    record.replace_initial_attempt(TxAttempt::active(
        attempt_parts(
            &txns[1],
            FeeRate::from_sat_per_vb(3).expect("test: valid fee rate"),
            Amount::from_sat(500),
        ),
        0,
    ));
    db.put_tx_node(node_id, record.clone()).unwrap();
    assert_eq!(db.get_active_tx_nodes().unwrap().len(), 1);

    // Retirement is guarded on the active txid the caller observed.
    assert!(!db
        .retire_tx_node(node_id, L1TxId::from([0xEE; 32]))
        .unwrap());
    assert_eq!(db.get_active_tx_nodes().unwrap().len(), 1);
    assert!(db.retire_tx_node(node_id, record.active_txid).unwrap());
    assert!(db.get_active_tx_nodes().unwrap().is_empty());

    // The retired record stays readable for point lookups, but its attempts drop their raw
    // transaction bytes: a retired chain never rebroadcasts, and keeping the bytes would grow
    // the database without bound.
    let retired = db
        .get_tx_node(node_id)
        .unwrap()
        .expect("retired record stays readable");
    assert!(retired
        .attempts
        .iter()
        .all(|attempt| attempt.raw_tx.is_empty()));
    record.forget_all_raw_txs();
    assert_eq!(retired, record);
}

// Helper function to generate a TxNodeRecord with a single active attempt
fn generate_tx_node_record(kind: TxNodeKind) -> TxNodeRecord {
    let txns = get_test_bitcoin_txs();
    let attempt = TxAttempt::active(
        attempt_parts(
            &txns[0],
            FeeRate::from_sat_per_vb(2).expect("test: valid fee rate"),
            Amount::from_sat(400),
        ),
        0,
    );
    TxNodeRecord::new(kind, attempt)
}

// Helper function to generate L1TxEntry
fn generate_l1_tx_entry() -> (Buf32, L1TxEntry) {
    let txns = get_test_bitcoin_txs();
    let txid = txns[0].compute_txid().as_raw_hash().to_byte_array().into();
    let txentry = tx_entry(&txns[0]);
    (txid, txentry)
}

fn get_test_bitcoin_txs() -> Vec<Transaction> {
    let tx_hex = [
        "0200000000010176f29f18c5fc677ad6dd6c9309f6b9112f83cb95889af21da4be7fbfe22d1d220000000000fdffffff0300e1f505000000002200203946555814a18ccc94ef4991fb6af45278425e6a0a2cfc2bf4cf9c47515c56ff0000000000000000176a1500e0e78c8201d91f362c2ad3bb6f8e6f31349454663b1010240100000022512012d77c9ae5fdca5a3ab0b17a29b683fd2690f5ad56f6057a000ec42081ac89dc0247304402205de15fbfb413505a3563608dad6a73eb271b4006a4156eeb62d1eacca5efa10b02201eb71b975304f3cbdc664c6dd1c07b93ac826603309b3258cb92cfd201bb8792012102f55f96fd587a706a7b5e7312c4e9d755a65b3dad9945d65598bca34c9e961db400000000",
        "02000000000101f4f2e8830d2948b5e980e739e61b23f048d03d4af81588bf5da4618406c495aa0000000000fdffffff02969e0700000000002200203946555814a18ccc94ef4991fb6af45278425e6a0a2cfc2bf4cf9c47515c56ff60f59000000000001600148d0499ec043b1921a608d24690b061196e57c927040047304402203875f7b610f8783d5f5c163118eeec1a23473dd33b53c8ea584c7d28a82b209b022034b6814344b79826a348e23cc19ff06ed2df23850b889557552e376bf9e32c560147304402200f647dad3c137ff98d7da7a302345c82a57116a3d0e6a3719293bbb421cb0abe02201c04a1e808f5bab3595f77985af91aeaf61e9e042c9ac97d696e0f4b020cb54b0169522102dba8352965522ff44538dde37d793b3b4ece54e07759ade5f648aa396165d2962103c0683712773b725e7fe4809cbc90c9e0b890c45e5e24a852a4c472d1b6e9fd482103bf56f172d0631a7f8ae3ef648ad43a816ad01de4137ba89ebc33a2da8c48531553ae00000000",
        "02000000000101f4f2e8830d2948b5e980e739e61b23f048d03d4af81588bf5da4618406c495aa0200000000ffffffff0380969800000000002200203946555814a18ccc94ef4991fb6af45278425e6a0a2cfc2bf4cf9c47515c56ff0000000000000000176a15006e1a916a60b93a545f2370f2a36d2f807fb3d675588b693a000000001600149fafc79c72d1c4d917a360f32bdc68755402ef670247304402203c813ad8918366ce872642368b57b78e78e03b1a1eafe16ec8f3c9268b4fc050022018affe880963f18bfc0338f1e54c970185aa90f8c36a52ac935fe76cb885d726012102fa9b81d082a98a46d0857d62e6c9afe9e1bf40f9f0cbf361b96241c9d6fb064b00000000",
        "02000000000101d8acf0a647b7d5d1d0ee83360158d5bf01146d3762c442defd7985476b02aa6b0100000000fdffffff030065cd1d000000002200203946555814a18ccc94ef4991fb6af45278425e6a0a2cfc2bf4cf9c47515c56ff0000000000000000176a1500e0e78c8201d91f362c2ad3bb6f8e6f3134945466aec19dd00000000022512040718748dbca6dea8ac6b6f0b177014f0826478f1613c2b489e738db7ecdf3610247304402207cfc5cd87ec83687c9ac2bd921e96b8a58710f15d77bc7624da4fb29fe589dab0220437b74ed8e8f9d3084269edfb8641bf27246b0e5476667918beba73025c7a2c501210249a34cfbb6163b1b6ca2fff63fd1f8a802fb1999fa7930b2febe5a711f713dd900000000",
        "0200000000010176f29f18c5fc677ad6dd6c9309f6b9112f83cb95889af21da4be7fbfe22d1d220000000000fdffffff0300e1f505000000002200203946555814a18ccc94ef4991fb6af45278425e6a0a2cfc2bf4cf9c47515c56ff0000000000000000176a1500e0e78c8201d91f362c2ad3bb6f8e6f31349454663b1010240100000022512012d77c9ae5fdca5a3ab0b17a29b683fd2690f5ad56f6057a000ec42081ac89dc0247304402205de15fbfb413505a3563608dad6a73eb271b4006a4156eeb62d1eacca5efa10b02201eb71b975304f3cbdc664c6dd1c07b93ac826603309b3258cb92cfd201bb8792012102f55f96fd587a706a7b5e7312c4e9d755a65b3dad9945d65598bca34c9e961db400000000",
    ];

    tx_hex
        .iter()
        .map(|encoded| {
            deserialize(&hex::decode(encoded).expect("valid test tx hex"))
                .expect("valid test tx bytes")
        })
        .collect()
}

#[macro_export]
macro_rules! l1_broadcast_db_tests {
    ($setup_expr:expr) => {
        #[test]
        fn test_get_last_tx_entry() {
            let db = $setup_expr;
            $crate::l1_broadcast_tests::test_get_last_tx_entry(&db);
        }

        #[test]
        fn test_add_tx_new_entry() {
            let db = $setup_expr;
            $crate::l1_broadcast_tests::test_add_tx_new_entry(&db);
        }

        #[test]
        fn test_put_tx_existing_entry() {
            let db = $setup_expr;
            $crate::l1_broadcast_tests::test_put_tx_existing_entry(&db);
        }

        #[test]
        fn test_put_tx_entry_pair() {
            let db = $setup_expr;
            $crate::l1_broadcast_tests::test_put_tx_entry_pair(&db);
        }

        #[test]
        fn test_update_tx_entry() {
            let db = $setup_expr;
            $crate::l1_broadcast_tests::test_update_tx_entry(&db);
        }

        #[test]
        fn test_update_tx_entry_rejects_mismatched_tx() {
            let db = $setup_expr;
            $crate::l1_broadcast_tests::test_update_tx_entry_rejects_mismatched_tx(&db);
        }

        #[test]
        fn test_get_txentry_by_idx() {
            let db = $setup_expr;
            $crate::l1_broadcast_tests::test_get_txentry_by_idx(&db);
        }

        #[test]
        fn test_get_next_txidx() {
            let db = $setup_expr;
            $crate::l1_broadcast_tests::test_get_next_txidx(&db);
        }

        #[test]
        fn test_del_tx_entry_single() {
            let db = $setup_expr;
            $crate::l1_broadcast_tests::test_del_tx_entry_single(&db);
        }

        #[test]
        fn test_del_tx_entries_from_idx() {
            let db = $setup_expr;
            $crate::l1_broadcast_tests::test_del_tx_entries_from_idx(&db);
        }

        #[test]
        fn test_del_tx_entries_empty_database() {
            let db = $setup_expr;
            $crate::l1_broadcast_tests::test_del_tx_entries_empty_database(&db);
        }

        #[test]
        fn test_put_replacement_tx_entry_refuses_existing_replacement() {
            let db = $setup_expr;
            $crate::l1_broadcast_tests::test_put_replacement_tx_entry_refuses_existing_replacement(
                &db,
            );
        }

        #[test]
        fn test_put_replacement_tx_entry_swaps_atomically() {
            let db = $setup_expr;
            $crate::l1_broadcast_tests::test_put_replacement_tx_entry_swaps_atomically(&db);
        }

        #[test]
        fn test_put_replacement_tx_entry_records_the_reverse_link() {
            let db = $setup_expr;
            $crate::l1_broadcast_tests::test_put_replacement_tx_entry_records_the_reverse_link(&db);
        }

        #[test]
        fn test_adopt_confirmed_ancestor_reverses_the_chain() {
            let db = $setup_expr;
            $crate::l1_broadcast_tests::test_adopt_confirmed_ancestor_reverses_the_chain(&db);
        }

        #[test]
        fn test_adopt_confirmed_ancestor_reverses_a_multi_hop_chain() {
            let db = $setup_expr;
            $crate::l1_broadcast_tests::test_adopt_confirmed_ancestor_reverses_a_multi_hop_chain(
                &db,
            );
        }

        #[test]
        fn test_adopt_confirmed_ancestor_refuses_a_superseded_loser() {
            let db = $setup_expr;
            $crate::l1_broadcast_tests::test_adopt_confirmed_ancestor_refuses_a_superseded_loser(
                &db,
            );
        }

        #[test]
        fn test_adopt_confirmed_ancestor_refuses_an_unlinked_pair() {
            let db = $setup_expr;
            $crate::l1_broadcast_tests::test_adopt_confirmed_ancestor_refuses_an_unlinked_pair(&db);
        }

        #[test]
        fn test_put_replacement_tx_entry_writes_nothing_when_refused() {
            let db = $setup_expr;
            $crate::l1_broadcast_tests::test_put_replacement_tx_entry_writes_nothing_when_refused(
                &db,
            );
        }

        #[test]
        fn test_try_mark_tx_entry_replaced_applies_to_published() {
            let db = $setup_expr;
            $crate::l1_broadcast_tests::test_try_mark_tx_entry_replaced_applies_to_published(&db);
        }

        #[test]
        fn test_try_mark_tx_entry_replaced_refuses_confirmed() {
            let db = $setup_expr;
            $crate::l1_broadcast_tests::test_try_mark_tx_entry_replaced_refuses_confirmed(&db);
        }

        #[test]
        fn test_try_mark_tx_entry_replaced_missing_entry() {
            let db = $setup_expr;
            $crate::l1_broadcast_tests::test_try_mark_tx_entry_replaced_missing_entry(&db);
        }

        #[test]
        fn test_put_tx_entry_by_idx_refuses_to_unreplace() {
            let db = $setup_expr;
            $crate::l1_broadcast_tests::test_put_tx_entry_by_idx_refuses_to_unreplace(&db);
        }

        #[test]
        fn test_put_tx_entry_by_idx_allows_rereplacement() {
            let db = $setup_expr;
            $crate::l1_broadcast_tests::test_put_tx_entry_by_idx_allows_rereplacement(&db);
        }

        #[test]
        fn test_tx_node_roundtrip() {
            let db = $setup_expr;
            $crate::l1_broadcast_tests::test_tx_node_roundtrip(&db);
        }

        #[test]
        fn test_tx_node_overwrite_and_list() {
            let db = $setup_expr;
            $crate::l1_broadcast_tests::test_tx_node_overwrite_and_list(&db);
        }

        #[test]
        fn test_active_tx_node_index_lifecycle() {
            let db = $setup_expr;
            $crate::l1_broadcast_tests::test_active_tx_node_index_lifecycle(&db);
        }
    };
}
