//! OL state indexing database tests.

use std::collections::BTreeMap;

use strata_db_types::{
    ol_state_index::{
        AccountUpdateMeta, AccountUpdateRecord, EpochIndexingData, InboxMessageRecord,
        IndexingWrites,
    },
    traits::OLStateIndexingDatabase,
    AccountCreatedRecord, DbError,
};
use strata_identifiers::{AccountId, Buf32, EpochCommitment, Hash, OLBlockCommitment, OLBlockId};

fn acct(seed: u8) -> AccountId {
    AccountId::new([seed; 32])
}

fn block(slot: u64, seed: u8) -> OLBlockCommitment {
    OLBlockCommitment::new(slot, OLBlockId::from(Buf32::from([seed; 32])))
}

fn hash(seed: u8) -> Hash {
    [seed; 32].into()
}

fn epoch_commit(epoch: u32, seed: u8) -> EpochCommitment {
    let slot = epoch as u64;
    EpochCommitment::new(epoch, slot, OLBlockId::from(Buf32::from([seed; 32])))
}

fn record(
    meta: Option<AccountUpdateMeta>,
    seq: u64,
    idx: u64,
    extra: Option<Vec<u8>>,
) -> AccountUpdateRecord {
    AccountUpdateRecord::new(meta, seq, idx, extra)
}

#[track_caller]
fn assert_duplicate_block(err: DbError, expected_epoch: u32, expected_block: OLBlockCommitment) {
    match err {
        DbError::DuplicateBlockIndexing { epoch, block } => {
            assert_eq!(epoch, expected_epoch, "duplicate error epoch mismatch");
            assert_eq!(block, expected_block, "duplicate error block mismatch");
        }
        other => panic!("expected DbError::DuplicateBlockIndexing, got {other:?}"),
    }
}

pub fn test_apply_epoch_indexing_round_trip(db: &impl OLStateIndexingDatabase) {
    let epoch = 7;
    let acct_a = acct(1);
    let acct_b = acct(2);

    let commitment = epoch_commit(epoch, 9);
    let mut updates = BTreeMap::new();
    updates.insert(acct_a, vec![record(None, 0, 0, Some(vec![1, 2, 3]))]);
    let mut inbox = BTreeMap::new();
    inbox.insert(acct_b, vec![InboxMessageRecord::new(vec![9, 9], None)]);

    let writes = IndexingWrites::new(vec![acct_a], updates, inbox);
    db.apply_epoch_indexing(commitment, writes)
        .expect("apply_epoch");

    // Checkpoint-sync has no per-block attribution; entries are tagged `None`.
    let expected = EpochIndexingData::new(
        Some(commitment),
        vec![AccountCreatedRecord::new_account(acct_a)],
        None,
    );
    assert_eq!(
        db.get_epoch_indexing_data(epoch).expect("get common"),
        Some(expected)
    );
    let got = db
        .get_account_update_records(epoch, acct_a)
        .expect("get update")
        .expect("present");
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].extra_data(), Some(&[1u8, 2, 3][..]));

    let inb = db
        .get_account_inbox_records(epoch, acct_b)
        .expect("get inbox")
        .expect("present");
    assert_eq!(inb.len(), 1);

    // Creation epoch index populated for created accounts.
    assert_eq!(
        db.get_account_creation_epoch(acct_a).expect("get creation"),
        Some(epoch)
    );
    assert!(db
        .get_account_creation_epoch(acct_b)
        .expect("get creation b")
        .is_none());
}

pub fn test_apply_block_indexing_appends(db: &impl OLStateIndexingDatabase) {
    let epoch = 3;
    let acct_a = acct(1);
    let block1 = block(10, 1);
    let block2 = block(11, 2);

    // Block 1: one update, one inbox write, account created.
    let mut updates1 = BTreeMap::new();
    updates1.insert(
        acct_a,
        vec![record(
            Some(AccountUpdateMeta::new(block1, hash(0x11))),
            1,
            5,
            Some(vec![0xAA]),
        )],
    );
    let mut inbox1 = BTreeMap::new();
    inbox1.insert(
        acct_a,
        vec![InboxMessageRecord::new(vec![0xA1], Some(block1))],
    );
    db.apply_block_indexing(
        epoch,
        block1,
        IndexingWrites::new(vec![acct_a], updates1, inbox1),
    )
    .expect("apply block1");

    // Block 2: two more updates, one inbox write, no new accounts.
    let mut updates2 = BTreeMap::new();
    updates2.insert(
        acct_a,
        vec![
            record(Some(AccountUpdateMeta::new(block2, hash(0x22))), 2, 6, None),
            record(
                Some(AccountUpdateMeta::new(block2, hash(0x33))),
                3,
                7,
                Some(vec![0xBB]),
            ),
        ],
    );
    let mut inbox2 = BTreeMap::new();
    inbox2.insert(
        acct_a,
        vec![InboxMessageRecord::new(vec![0xA2], Some(block2))],
    );
    db.apply_block_indexing(epoch, block2, IndexingWrites::new(vec![], updates2, inbox2))
        .expect("apply block2");

    let got = db
        .get_account_update_records(epoch, acct_a)
        .expect("get update")
        .expect("present");
    assert_eq!(got.len(), 3);
    assert_eq!(got[0].seq_no(), 1);
    assert_eq!(got[1].seq_no(), 2);
    assert_eq!(got[2].seq_no(), 3);

    let inb = db
        .get_account_inbox_records(epoch, acct_a)
        .expect("get inbox")
        .expect("present");
    assert_eq!(inb.len(), 2);

    let common = db
        .get_epoch_indexing_data(epoch)
        .expect("get common")
        .expect("present");
    assert_eq!(
        common.created_accounts(),
        &[AccountCreatedRecord::new(acct_a, Some(block1))]
    );
    assert!(common.epoch_commitment().is_none());

    assert_eq!(
        db.get_account_creation_epoch(acct_a).expect("get creation"),
        Some(epoch)
    );
}

pub fn test_set_epoch_commitment_stamps(db: &impl OLStateIndexingDatabase) {
    let epoch = 5;
    let acct_a = acct(1);
    let blk = block(1, 1);

    db.apply_block_indexing(
        epoch,
        blk,
        IndexingWrites::new(vec![acct_a], BTreeMap::new(), BTreeMap::new()),
    )
    .expect("apply block");

    let commitment = epoch_commit(epoch, 5);
    db.set_epoch_commitment(epoch, commitment)
        .expect("set commitment");

    let common = db
        .get_epoch_indexing_data(epoch)
        .expect("get common")
        .expect("present");
    assert_eq!(common.epoch_commitment(), Some(&commitment));
    // Created accounts preserved.
    assert_eq!(
        common.created_accounts(),
        &[AccountCreatedRecord::new(acct_a, Some(blk))]
    );
}

pub fn test_set_epoch_commitment_missing_row_errors(db: &impl OLStateIndexingDatabase) {
    let result = db.set_epoch_commitment(99, epoch_commit(99, 0));
    assert!(result.is_err(), "expected error for missing common row");
}

pub fn test_get_account_creation_epoch_absent(db: &impl OLStateIndexingDatabase) {
    let got = db
        .get_account_creation_epoch(acct(42))
        .expect("get creation");
    assert!(got.is_none());
}

pub fn test_account_active_in_multiple_epochs_creation_epoch_unchanged(
    db: &impl OLStateIndexingDatabase,
) {
    let acct_a = acct(1);
    // Created in epoch 2.
    db.apply_block_indexing(
        2,
        block(1, 1),
        IndexingWrites::new(vec![acct_a], BTreeMap::new(), BTreeMap::new()),
    )
    .expect("apply epoch 2");
    // Activity in epoch 3 must not overwrite creation epoch.
    let mut updates = BTreeMap::new();
    updates.insert(
        acct_a,
        vec![record(
            Some(AccountUpdateMeta::new(block(2, 2), hash(1))),
            0,
            0,
            None,
        )],
    );
    db.apply_block_indexing(
        3,
        block(2, 2),
        IndexingWrites::new(vec![], updates, BTreeMap::new()),
    )
    .expect("apply epoch 3");

    assert_eq!(
        db.get_account_creation_epoch(acct_a).expect("get creation"),
        Some(2)
    );
}

pub fn test_apply_block_indexing_duplicate_block_errors(db: &impl OLStateIndexingDatabase) {
    let epoch = 4;
    let acct_a = acct(1);
    let blk = block(7, 7);

    let mut updates = BTreeMap::new();
    updates.insert(
        acct_a,
        vec![record(
            Some(AccountUpdateMeta::new(blk, hash(0xAA))),
            1,
            5,
            Some(vec![0xAA]),
        )],
    );

    let writes = || IndexingWrites::new(vec![], updates.clone(), BTreeMap::new());

    db.apply_block_indexing(epoch, blk, writes())
        .expect("first apply");

    let err = db
        .apply_block_indexing(epoch, blk, writes())
        .expect_err("duplicate apply should error");
    assert_duplicate_block(err, epoch, blk);

    // Original record is intact and not duplicated.
    let got = db
        .get_account_update_records(epoch, acct_a)
        .expect("get entry")
        .expect("present");
    assert_eq!(got.len(), 1);
}

/// Re-applying a block that wrote only inbox records (no updates, no creators)
/// must still abort. Pre-`last_applied_block` design missed this case.
pub fn test_apply_block_indexing_dedup_inbox_only(db: &impl OLStateIndexingDatabase) {
    let epoch = 12;
    let acct_a = acct(1);
    let blk = block(20, 1);

    let mut inbox = BTreeMap::new();
    inbox.insert(acct_a, vec![InboxMessageRecord::new(vec![0xA1], Some(blk))]);
    let writes = || IndexingWrites::new(vec![], BTreeMap::new(), inbox.clone());

    db.apply_block_indexing(epoch, blk, writes())
        .expect("first");

    let err = db
        .apply_block_indexing(epoch, blk, writes())
        .expect_err("inbox-only re-apply must error");
    assert_duplicate_block(err, epoch, blk);

    // Original inbox record still single, not duplicated.
    let got = db
        .get_account_inbox_records(epoch, acct_a)
        .expect("get inbox")
        .expect("present");
    assert_eq!(got.len(), 1);
}

/// Re-applying a block that wrote only created_accounts (no updates, no inbox)
/// must still abort. Pre-`last_applied_block` design missed this case.
pub fn test_apply_block_indexing_dedup_creators_only(db: &impl OLStateIndexingDatabase) {
    let epoch = 13;
    let acct_a = acct(1);
    let blk = block(21, 1);

    let writes = || IndexingWrites::new(vec![acct_a], BTreeMap::new(), BTreeMap::new());

    db.apply_block_indexing(epoch, blk, writes())
        .expect("first");

    let err = db
        .apply_block_indexing(epoch, blk, writes())
        .expect_err("creators-only re-apply must error");
    assert_duplicate_block(err, epoch, blk);

    // created_accounts has exactly one entry, not duplicated.
    let common = db
        .get_epoch_indexing_data(epoch)
        .expect("get common")
        .expect("present");
    assert_eq!(
        common.created_accounts(),
        &[AccountCreatedRecord::new(acct_a, Some(blk))]
    );
}

/// Applying out-of-order (later slot then earlier slot in the same epoch)
/// must abort. The high-water mark contract demands monotonic apply.
pub fn test_apply_block_indexing_rejects_out_of_order(db: &impl OLStateIndexingDatabase) {
    let epoch = 14;
    let acct_a = acct(1);
    let blk_high = block(30, 1);
    let blk_low = block(20, 2);

    db.apply_block_indexing(
        epoch,
        blk_high,
        IndexingWrites::new(vec![acct_a], BTreeMap::new(), BTreeMap::new()),
    )
    .expect("apply high");

    let err = db
        .apply_block_indexing(
            epoch,
            blk_low,
            IndexingWrites::new(vec![], BTreeMap::new(), BTreeMap::new()),
        )
        .expect_err("out-of-order apply must error");
    assert_duplicate_block(err, epoch, blk_low);
}

/// After rollback_to_block, the high-water mark must allow re-applying the
/// dropped slots' successors.
pub fn test_rollback_to_block_resets_high_water(db: &impl OLStateIndexingDatabase) {
    let epoch = 15;
    let acct_a = acct(1);
    let blk10 = block(10, 1);
    let blk11 = block(11, 2);

    db.apply_block_indexing(
        epoch,
        blk10,
        IndexingWrites::new(vec![acct_a], BTreeMap::new(), BTreeMap::new()),
    )
    .expect("apply 10");
    db.apply_block_indexing(
        epoch,
        blk11,
        IndexingWrites::new(vec![], BTreeMap::new(), BTreeMap::new()),
    )
    .expect("apply 11");

    db.rollback_to_block(epoch, blk10).expect("rollback");

    // The high-water was at slot 11; rollback should reset it so slot 11
    // can be re-applied with a different blkid (e.g. fork after reorg).
    let blk11_new = block(11, 99);
    db.apply_block_indexing(
        epoch,
        blk11_new,
        IndexingWrites::new(vec![], BTreeMap::new(), BTreeMap::new()),
    )
    .expect("re-apply 11 succeeds");
}

/// Apply two blocks in an epoch; rollback to block1 should drop block2's
/// data while keeping block1 intact.
pub fn test_rollback_to_block_drops_later_blocks(db: &impl OLStateIndexingDatabase) {
    let epoch = 5;
    let acct_a = acct(1);
    let acct_b = acct(2);
    let blk1 = block(10, 1);
    let blk2 = block(11, 2);

    let mut u1 = BTreeMap::new();
    u1.insert(
        acct_a,
        vec![record(
            Some(AccountUpdateMeta::new(blk1, hash(1))),
            1,
            5,
            Some(vec![0xAA]),
        )],
    );
    db.apply_block_indexing(
        epoch,
        blk1,
        IndexingWrites::new(vec![acct_a], u1, BTreeMap::new()),
    )
    .expect("apply blk1");

    let mut u2 = BTreeMap::new();
    u2.insert(
        acct_a,
        vec![record(
            Some(AccountUpdateMeta::new(blk2, hash(2))),
            2,
            6,
            Some(vec![0xBB]),
        )],
    );
    let mut i2 = BTreeMap::new();
    i2.insert(
        acct_a,
        vec![InboxMessageRecord::new(vec![0xA2], Some(blk2))],
    );
    db.apply_block_indexing(epoch, blk2, IndexingWrites::new(vec![acct_b], u2, i2))
        .expect("apply blk2");

    db.rollback_to_block(epoch, blk1).expect("rollback");

    // Only block1's update for acct_a is left.
    let got = db
        .get_account_update_records(epoch, acct_a)
        .expect("get update")
        .expect("present");
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].seq_no(), 1);

    // block2's inbox write gone.
    assert!(db
        .get_account_inbox_records(epoch, acct_a)
        .expect("get inbox")
        .is_none());

    // acct_a creation kept (created in blk1, slot <= cutoff); acct_b dropped.
    assert_eq!(
        db.get_account_creation_epoch(acct_a)
            .expect("get creation a"),
        Some(epoch)
    );
    assert!(db
        .get_account_creation_epoch(acct_b)
        .expect("get creation b")
        .is_none());

    let common = db
        .get_epoch_indexing_data(epoch)
        .expect("get common")
        .expect("present");
    assert_eq!(
        common.created_accounts(),
        &[AccountCreatedRecord::new(acct_a, Some(blk1))]
    );
}

/// Rolling back to a block with slot >= every applied block keeps
/// everything intact.
pub fn test_rollback_to_block_keeps_when_at_or_past_tip(db: &impl OLStateIndexingDatabase) {
    let epoch = 6;
    let acct_a = acct(1);
    let blk = block(10, 1);
    let mut u = BTreeMap::new();
    u.insert(
        acct_a,
        vec![record(
            Some(AccountUpdateMeta::new(blk, hash(1))),
            1,
            5,
            Some(vec![0xAA]),
        )],
    );
    db.apply_block_indexing(
        epoch,
        blk,
        IndexingWrites::new(vec![acct_a], u, BTreeMap::new()),
    )
    .expect("apply");

    // Cutoff at blk's own slot keeps blk (cutoff is inclusive).
    db.rollback_to_block(epoch, blk).expect("rollback");

    let got = db
        .get_account_update_records(epoch, acct_a)
        .expect("get update")
        .expect("present");
    assert_eq!(got.len(), 1);
    assert_eq!(
        db.get_account_creation_epoch(acct_a).expect("get creation"),
        Some(epoch)
    );
}

pub fn test_rollback_to_block_idempotent(db: &impl OLStateIndexingDatabase) {
    let epoch = 7;
    let blk_at = block(5, 0);

    // No prior apply: rollback is a silent no-op.
    db.rollback_to_block(epoch, blk_at).expect("first");
    db.rollback_to_block(epoch, blk_at).expect("second");

    let acct_a = acct(1);
    let blk_later = block(10, 1);
    let mut u = BTreeMap::new();
    u.insert(
        acct_a,
        vec![record(
            Some(AccountUpdateMeta::new(blk_later, hash(1))),
            1,
            5,
            Some(vec![0xAA]),
        )],
    );
    db.apply_block_indexing(
        epoch,
        blk_later,
        IndexingWrites::new(vec![acct_a], u, BTreeMap::new()),
    )
    .expect("apply");

    db.rollback_to_block(epoch, blk_at).expect("rb1");
    db.rollback_to_block(epoch, blk_at).expect("rb2");
    assert!(db
        .get_account_update_records(epoch, acct_a)
        .expect("get update")
        .is_none());
    assert!(db
        .get_account_creation_epoch(acct_a)
        .expect("get creation")
        .is_none());
}

pub fn test_rollback_to_block_preserves_commitment(db: &impl OLStateIndexingDatabase) {
    let epoch = 8;
    let acct_a = acct(1);
    let blk = block(20, 1);
    let commitment = epoch_commit(epoch, 9);

    db.apply_block_indexing(
        epoch,
        blk,
        IndexingWrites::new(vec![acct_a], BTreeMap::new(), BTreeMap::new()),
    )
    .expect("apply");
    db.set_epoch_commitment(epoch, commitment).expect("set");

    // Rollback to slot 0 drops the only block-tagged creator.
    db.rollback_to_block(epoch, block(0, 0)).expect("rollback");

    let common = db
        .get_epoch_indexing_data(epoch)
        .expect("get common")
        .expect("present");
    assert_eq!(common.epoch_commitment(), Some(&commitment));
    assert!(common.created_accounts().is_empty());
}

/// Per-block rollback should never drop checkpoint-sync (`None`) creators.
pub fn test_rollback_to_block_immune_to_checkpoint_sync(db: &impl OLStateIndexingDatabase) {
    let epoch = 9;
    let acct_a = acct(1);
    let commitment = epoch_commit(epoch, 9);

    db.apply_epoch_indexing(
        commitment,
        IndexingWrites::new(vec![acct_a], BTreeMap::new(), BTreeMap::new()),
    )
    .expect("apply epoch");

    db.rollback_to_block(epoch, block(0, 0)).expect("rollback");

    let common = db
        .get_epoch_indexing_data(epoch)
        .expect("get common")
        .expect("present");
    assert_eq!(
        common.created_accounts(),
        &[AccountCreatedRecord::new(acct_a, None)]
    );
    assert_eq!(
        db.get_account_creation_epoch(acct_a).expect("get creation"),
        Some(epoch)
    );
}

/// Apply data in epochs 5, 6, 7. Roll back to epoch 5: keeps 5, drops 6 and 7.
pub fn test_rollback_to_epoch_drops_later_epochs(db: &impl OLStateIndexingDatabase) {
    let acct_a = acct(1);
    for ep in [5u32, 6, 7] {
        let blk = block(ep as u64, ep as u8);
        let mut u = BTreeMap::new();
        u.insert(
            acct_a,
            vec![record(
                Some(AccountUpdateMeta::new(blk, hash(ep as u8))),
                1,
                5,
                Some(vec![0xAA]),
            )],
        );
        let creators = if ep == 5 { vec![acct_a] } else { vec![] };
        db.apply_block_indexing(ep, blk, IndexingWrites::new(creators, u, BTreeMap::new()))
            .expect("apply");
    }

    db.rollback_to_epoch(5).expect("rollback");

    // Epoch 5 intact.
    assert!(db.get_epoch_indexing_data(5).expect("get 5").is_some());
    assert!(db
        .get_account_update_records(5, acct_a)
        .expect("get a@5")
        .is_some());
    assert_eq!(
        db.get_account_creation_epoch(acct_a).expect("get creation"),
        Some(5)
    );

    // Epochs 6, 7 wiped.
    for ep in [6u32, 7] {
        assert!(db.get_epoch_indexing_data(ep).expect("get").is_none());
        assert!(db
            .get_account_update_records(ep, acct_a)
            .expect("get a")
            .is_none());
    }
}

pub fn test_rollback_to_epoch_idempotent(db: &impl OLStateIndexingDatabase) {
    // No-op on empty store.
    db.rollback_to_epoch(99).expect("first");
    db.rollback_to_epoch(99).expect("second");

    // Apply once at higher epoch, rollback twice.
    let acct_a = acct(1);
    let blk = block(50, 1);
    db.apply_block_indexing(
        10,
        blk,
        IndexingWrites::new(vec![acct_a], BTreeMap::new(), BTreeMap::new()),
    )
    .expect("apply");
    db.rollback_to_epoch(5).expect("rb1");
    db.rollback_to_epoch(5).expect("rb2");
    assert!(db.get_epoch_indexing_data(10).expect("get").is_none());
    assert!(db
        .get_account_creation_epoch(acct_a)
        .expect("get creation")
        .is_none());
}

#[macro_export]
macro_rules! ol_state_indexing_db_tests {
    ($setup_expr:expr) => {
        #[test]
        fn test_apply_epoch_indexing_round_trip() {
            let db = $setup_expr;
            $crate::ol_state_indexing_tests::test_apply_epoch_indexing_round_trip(&db);
        }

        #[test]
        fn test_apply_block_indexing_appends() {
            let db = $setup_expr;
            $crate::ol_state_indexing_tests::test_apply_block_indexing_appends(&db);
        }

        #[test]
        fn test_set_epoch_commitment_stamps() {
            let db = $setup_expr;
            $crate::ol_state_indexing_tests::test_set_epoch_commitment_stamps(&db);
        }

        #[test]
        fn test_set_epoch_commitment_missing_row_errors() {
            let db = $setup_expr;
            $crate::ol_state_indexing_tests::test_set_epoch_commitment_missing_row_errors(&db);
        }

        #[test]
        fn test_get_account_creation_epoch_absent() {
            let db = $setup_expr;
            $crate::ol_state_indexing_tests::test_get_account_creation_epoch_absent(&db);
        }

        #[test]
        fn test_account_active_in_multiple_epochs_creation_epoch_unchanged() {
            let db = $setup_expr;
            $crate::ol_state_indexing_tests::test_account_active_in_multiple_epochs_creation_epoch_unchanged(&db);
        }

        #[test]
        fn test_apply_block_indexing_duplicate_block_errors() {
            let db = $setup_expr;
            $crate::ol_state_indexing_tests::test_apply_block_indexing_duplicate_block_errors(&db);
        }

        #[test]
        fn test_apply_block_indexing_dedup_inbox_only() {
            let db = $setup_expr;
            $crate::ol_state_indexing_tests::test_apply_block_indexing_dedup_inbox_only(&db);
        }

        #[test]
        fn test_apply_block_indexing_dedup_creators_only() {
            let db = $setup_expr;
            $crate::ol_state_indexing_tests::test_apply_block_indexing_dedup_creators_only(&db);
        }

        #[test]
        fn test_apply_block_indexing_rejects_out_of_order() {
            let db = $setup_expr;
            $crate::ol_state_indexing_tests::test_apply_block_indexing_rejects_out_of_order(&db);
        }

        #[test]
        fn test_rollback_to_block_resets_high_water() {
            let db = $setup_expr;
            $crate::ol_state_indexing_tests::test_rollback_to_block_resets_high_water(&db);
        }

        #[test]
        fn test_rollback_to_block_drops_later_blocks() {
            let db = $setup_expr;
            $crate::ol_state_indexing_tests::test_rollback_to_block_drops_later_blocks(&db);
        }

        #[test]
        fn test_rollback_to_block_keeps_when_at_or_past_tip() {
            let db = $setup_expr;
            $crate::ol_state_indexing_tests::test_rollback_to_block_keeps_when_at_or_past_tip(&db);
        }

        #[test]
        fn test_rollback_to_block_idempotent() {
            let db = $setup_expr;
            $crate::ol_state_indexing_tests::test_rollback_to_block_idempotent(&db);
        }

        #[test]
        fn test_rollback_to_block_preserves_commitment() {
            let db = $setup_expr;
            $crate::ol_state_indexing_tests::test_rollback_to_block_preserves_commitment(&db);
        }

        #[test]
        fn test_rollback_to_block_immune_to_checkpoint_sync() {
            let db = $setup_expr;
            $crate::ol_state_indexing_tests::test_rollback_to_block_immune_to_checkpoint_sync(&db);
        }

        #[test]
        fn test_rollback_to_epoch_drops_later_epochs() {
            let db = $setup_expr;
            $crate::ol_state_indexing_tests::test_rollback_to_epoch_drops_later_epochs(&db);
        }

        #[test]
        fn test_rollback_to_epoch_idempotent() {
            let db = $setup_expr;
            $crate::ol_state_indexing_tests::test_rollback_to_epoch_idempotent(&db);
        }
    };
}
