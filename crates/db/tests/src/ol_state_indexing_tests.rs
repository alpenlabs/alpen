//! OL state indexing database tests.

use std::collections::BTreeMap;

use strata_db_types::{
    ol_state_index::{
        AccountEpochKey, AccountInboxEntry, AccountUpdateEntry, AccountUpdateMeta,
        AccountUpdateRecord, BlockIndexingWrites, EpochIndexingData, EpochIndexingWrites,
        InboxMessageRecord,
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

    let common = EpochIndexingData::new(Some(epoch_commit(epoch as u32, 9)), vec![acct_a]);
    let mut updates = BTreeMap::new();
    updates.insert(
        acct_a,
        AccountUpdateEntry::new(vec![record(None, 0, 0, Some(vec![1, 2, 3]))]),
    );
    let mut inbox = BTreeMap::new();
    inbox.insert(
        acct_b,
        AccountInboxEntry::new(vec![InboxMessageRecord::new(vec![9, 9], None)]),
    );

    let writes = EpochIndexingWrites {
        epoch,
        common: common.clone(),
        account_updates: updates,
        account_inbox: inbox,
    };
    db.apply_epoch_indexing(writes).expect("apply_epoch");

    assert_eq!(
        db.get_epoch_indexing_data(epoch).expect("get common"),
        Some(common)
    );
    let got = db
        .get_account_update_entry(AccountEpochKey::new(epoch, acct_a))
        .expect("get update")
        .expect("present");
    assert_eq!(got.records().len(), 1);
    assert_eq!(got.records()[0].extra_data(), Some(&[1u8, 2, 3][..]));

    let inb = db
        .get_account_inbox_entry(AccountEpochKey::new(epoch, acct_b))
        .expect("get inbox")
        .expect("present");
    assert_eq!(inb.records().len(), 1);

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
    db.apply_block_indexing(BlockIndexingWrites {
        epoch,
        block: block1,
        created_accounts: vec![acct_a],
        account_updates: updates1,
        account_inbox_writes: inbox1,
    })
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
    db.apply_block_indexing(BlockIndexingWrites {
        epoch,
        block: block2,
        created_accounts: vec![],
        account_updates: updates2,
        account_inbox_writes: inbox2,
    })
    .expect("apply block2");

    let got = db
        .get_account_update_entry(AccountEpochKey::new(epoch, acct_a))
        .expect("get update")
        .expect("present");
    assert_eq!(got.records().len(), 3);
    assert_eq!(got.records()[0].seq_no(), 1);
    assert_eq!(got.records()[1].seq_no(), 2);
    assert_eq!(got.records()[2].seq_no(), 3);

    let inb = db
        .get_account_inbox_entry(AccountEpochKey::new(epoch, acct_a))
        .expect("get inbox")
        .expect("present");
    assert_eq!(inb.records().len(), 2);

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

    db.apply_block_indexing(BlockIndexingWrites {
        epoch,
        block: block(1, 1),
        created_accounts: vec![acct_a],
        account_updates: BTreeMap::new(),
        account_inbox_writes: BTreeMap::new(),
    })
    .expect("apply block");

    let commitment = epoch_commit(epoch as u32, 5);
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
    db.apply_block_indexing(BlockIndexingWrites {
        epoch: 2,
        block: block(1, 1),
        created_accounts: vec![acct_a],
        account_updates: BTreeMap::new(),
        account_inbox_writes: BTreeMap::new(),
    })
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
    db.apply_block_indexing(BlockIndexingWrites {
        epoch: 3,
        block: block(2, 2),
        created_accounts: vec![],
        account_updates: updates,
        account_inbox_writes: BTreeMap::new(),
    })
    .expect("apply epoch 3");

    assert_eq!(
        db.get_account_creation_epoch(acct_a).expect("get creation"),
        Some(2)
    );
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
    };
}
