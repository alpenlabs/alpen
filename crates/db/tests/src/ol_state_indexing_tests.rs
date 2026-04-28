//! OL state indexing database tests.

use std::collections::BTreeMap;

use strata_db_types::{
    ol_state_index::{
        AccountUpdateMeta, AccountUpdateRecord, EpochIndexingData, InboxMessageRecord,
        IndexingWrites,
    },
    traits::OLStateIndexingDatabase,
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

    let expected = EpochIndexingData::new(Some(commitment), vec![acct_a]);
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
    assert_eq!(common.created_accounts(), &[acct_a]);
    assert!(common.epoch_commitment().is_none());

    assert_eq!(
        db.get_account_creation_epoch(acct_a).expect("get creation"),
        Some(epoch)
    );
}

pub fn test_set_epoch_commitment_stamps(db: &impl OLStateIndexingDatabase) {
    let epoch = 5;
    let acct_a = acct(1);

    db.apply_block_indexing(
        epoch,
        block(1, 1),
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
    assert_eq!(common.created_accounts(), &[acct_a]);
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
    let msg = err.to_string();
    assert!(
        msg.contains("already indexed"),
        "expected DuplicateBlockIndexing-shaped error, got: {msg}"
    );

    // Original record is intact and not duplicated.
    let got = db
        .get_account_update_records(epoch, acct_a)
        .expect("get entry")
        .expect("present");
    assert_eq!(got.len(), 1);
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
    };
}
