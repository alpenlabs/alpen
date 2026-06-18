//! One-shot, idempotent schema migration for the OL sled database.
//!
//! Upgrades an OL `<datadir>/sled/<dbname>` written by pre-#1849 binaries so
//! current binaries can read it. The affected record is the
//! `OLAccountUpdateEntrySchema` value (`Vec<AccountUpdateRecord>`, CBOR via
//! ciborium): #1942 inserted `prev_next_inbox_idx: u64` between `seq_no` and
//! `next_inbox_idx`. ciborium maps are keyed by field name and a missing
//! required field fails to decode, so this is a real decode break.
//!
//! Backfill rule (see the doc comment on [`AccountUpdateRecord`] describing the
//! consumed-message range `[prev_next_inbox_idx, next_inbox_idx)`):
//! within a stored `Vec`, `record[i].prev_next_inbox_idx =
//! record[i-1].next_inbox_idx`; the first record of an epoch chains from the
//! prior epoch's terminal `next_inbox_idx` (per account). Accounts are
//! processed independently; epochs are walked ascending so cross-epoch chaining
//! is correct.
//!
//! Raw sled keys are preserved: the key is decoded only to obtain
//! `(account_id, epoch)` ordering, never re-encoded.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use strata_db_types::ol_state_index::{AccountEpochKey, AccountUpdateMeta, AccountUpdateRecord};
use strata_identifiers::{AccountId, Epoch, Hash, OLBlockCommitment};
use tracing::{info, warn};
use typed_sled::codec::KeyCodec;

use crate::ol_state_index::schemas::OLAccountUpdateEntrySchema;

/// Key holding the OL schema version marker (in the default tree).
const VERSION_KEY: &[u8] = b"alpen_schema_version";
/// Schema version this migration produces.
pub const OL_SCHEMA_VERSION: u32 = 1;

const ACCOUNT_UPDATE_TREE: &str = "OLAccountUpdateEntrySchema";

/// Mirror of `AccountUpdateMeta` (unchanged by #1942), for V0 CBOR decode.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct AccountUpdateMetaV0 {
    block_commitment: Option<OLBlockCommitment>,
    new_state_root: Hash,
}

impl AccountUpdateMetaV0 {
    fn into_current(self) -> AccountUpdateMeta {
        AccountUpdateMeta::new(self.block_commitment, self.new_state_root)
    }
}

/// V0 (pre-#1942) `AccountUpdateRecord`: no `prev_next_inbox_idx`.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct AccountUpdateRecordV0 {
    update_meta: Option<AccountUpdateMetaV0>,
    seq_no: u64,
    next_inbox_idx: u64,
    extra_data: Option<Vec<u8>>,
}

/// Summary of what the OL migration touched.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct OlMigrationReport {
    /// Whether the migration ran (false = already at target version, no-op).
    pub ran: bool,
    /// `OLAccountUpdateEntrySchema` rows rewritten.
    pub account_update_rows_migrated: u64,
    /// Total `AccountUpdateRecord`s rewritten across all rows.
    pub account_update_records_migrated: u64,
}

/// Reads the OL schema version marker from the default tree.
fn read_version(db: &sled::Db) -> anyhow::Result<Option<u32>> {
    use anyhow::Context;
    let raw = db
        .get(VERSION_KEY)
        .context("reading OL schema version marker")?;
    match raw {
        None => Ok(None),
        Some(bytes) => {
            let arr: [u8; 4] = bytes
                .as_ref()
                .try_into()
                .map_err(|_| anyhow::anyhow!("OL schema version marker has unexpected length"))?;
            Ok(Some(u32::from_le_bytes(arr)))
        }
    }
}

/// Writes the OL schema version marker into the default tree.
fn write_version(db: &sled::Db, version: u32) -> anyhow::Result<()> {
    use anyhow::Context;
    db.insert(VERSION_KEY, &version.to_le_bytes())
        .context("writing OL schema version marker")?;
    db.flush().context("flushing OL schema version marker")?;
    Ok(())
}

/// One row staged for rewrite: raw key + the parsed `(epoch, records)`.
struct StagedRow {
    raw_key: sled::IVec,
    epoch: Epoch,
    records: Vec<AccountUpdateRecordV0>,
}

/// Runs the OL schema migration on an already-open sled database, idempotently.
///
/// Safe to call repeatedly: if the version marker already equals
/// [`OL_SCHEMA_VERSION`], it returns without touching any tree.
pub fn migrate_ol_db(db: &sled::Db) -> anyhow::Result<OlMigrationReport> {
    use anyhow::Context;

    if read_version(db)?.is_some_and(|v| v >= OL_SCHEMA_VERSION) {
        info!(
            version = OL_SCHEMA_VERSION,
            "OL DB already at target schema version; skipping"
        );
        return Ok(OlMigrationReport::default());
    }

    let tree = db
        .open_tree(ACCOUNT_UPDATE_TREE)
        .context("open OLAccountUpdateEntrySchema")?;

    // Group rows by account, decoding keys for ordering only.
    let mut by_account: BTreeMap<AccountId, Vec<StagedRow>> = BTreeMap::new();
    for kv in tree.iter() {
        let (k, v) = kv.context("iter OLAccountUpdateEntrySchema")?;

        // Already current layout? Skip (defensive; supports partial reruns).
        if ciborium::from_reader::<Vec<AccountUpdateRecord>, _>(v.as_ref()).is_ok() {
            continue;
        }

        let key = <AccountEpochKey as KeyCodec<OLAccountUpdateEntrySchema>>::decode_key(k.as_ref())
            .map_err(|e| anyhow::anyhow!("decode AccountEpochKey: {e}"))?;
        let records: Vec<AccountUpdateRecordV0> = ciborium::from_reader(v.as_ref())
            .map_err(|e| anyhow::anyhow!("decode V0 Vec<AccountUpdateRecord> (cbor): {e}"))?;

        by_account.entry(key.account_id()).or_default().push(StagedRow {
            raw_key: k,
            epoch: key.epoch(),
            records,
        });
    }

    let mut rows_migrated = 0u64;
    let mut records_migrated = 0u64;

    for (account_id, mut rows) in by_account {
        // Walk epochs ascending so cross-epoch chaining is correct.
        rows.sort_by_key(|r| r.epoch);

        // Running terminal `next_inbox_idx` carried across epochs. The very
        // first record (first epoch present for this account) chains from 0.
        let mut prev_terminal: u64 = 0;

        for row in rows {
            let mut new_records = Vec::with_capacity(row.records.len());
            for v0 in row.records {
                let prev_next_inbox_idx = prev_terminal;
                let rec = AccountUpdateRecord::new(
                    v0.update_meta.map(AccountUpdateMetaV0::into_current),
                    v0.seq_no,
                    prev_next_inbox_idx,
                    v0.next_inbox_idx,
                    v0.extra_data,
                );
                prev_terminal = v0.next_inbox_idx;
                new_records.push(rec);
            }
            records_migrated += new_records.len() as u64;

            let mut buf = Vec::new();
            ciborium::into_writer(&new_records, &mut buf)
                .map_err(|e| anyhow::anyhow!("encode current Vec<AccountUpdateRecord> (cbor): {e}"))?;
            tree.insert(&row.raw_key, buf)
                .context("write migrated account-update row")?;
            rows_migrated += 1;
        }
        let _ = account_id;
    }

    tree.flush().context("flush OLAccountUpdateEntrySchema")?;
    write_version(db, OL_SCHEMA_VERSION)?;

    if rows_migrated == 0 {
        warn!("OL migration ran but rewrote no rows (all already current?)");
    }
    info!(
        rows_migrated,
        records_migrated, "OL DB schema migration complete"
    );

    Ok(OlMigrationReport {
        ran: true,
        account_update_rows_migrated: rows_migrated,
        account_update_records_migrated: records_migrated,
    })
}

#[cfg(test)]
mod tests {
    use sled::Config as SledConfig;

    use super::*;

    fn temp_db() -> sled::Db {
        SledConfig::new().temporary(true).open().expect("temp sled")
    }

    fn acct(n: u8) -> AccountId {
        AccountId::new([n; 32])
    }

    /// Encodes the raw key bytes the schema uses for `(epoch, account)`.
    fn key_bytes(epoch: Epoch, account: AccountId) -> Vec<u8> {
        <AccountEpochKey as KeyCodec<OLAccountUpdateEntrySchema>>::encode_key(
            &AccountEpochKey::new(epoch, account),
        )
        .expect("encode key")
    }

    /// Builds a V0 record with the given `next_inbox_idx` and CBOR-encodes a Vec.
    fn v0_row(next_idxs: &[u64]) -> Vec<u8> {
        let recs: Vec<AccountUpdateRecordV0> = next_idxs
            .iter()
            .enumerate()
            .map(|(i, &n)| AccountUpdateRecordV0 {
                update_meta: None,
                seq_no: i as u64 + 1,
                next_inbox_idx: n,
                extra_data: None,
            })
            .collect();
        let mut buf = Vec::new();
        ciborium::into_writer(&recs, &mut buf).expect("encode v0");
        buf
    }

    /// End-to-end OL migration: a single account with three epoch rows whose
    /// `next_inbox_idx` values chain across epochs; assert the backfilled
    /// `prev_next_inbox_idx` is correct (not 0) and the current struct decodes.
    #[test]
    fn migrate_ol_db_backfills_prev_next_inbox_idx() {
        let db = temp_db();
        let tree = db.open_tree(ACCOUNT_UPDATE_TREE).unwrap();
        let a = acct(1);

        // epoch 0: next=[2,5]; epoch 1: next=[9]; epoch 2: next=[9,11]
        tree.insert(key_bytes(0, a), v0_row(&[2, 5])).unwrap();
        tree.insert(key_bytes(1, a), v0_row(&[9])).unwrap();
        tree.insert(key_bytes(2, a), v0_row(&[9, 11])).unwrap();

        // Sanity: current decode fails pre-migration.
        for epoch in 0..3 {
            let v = tree.get(key_bytes(epoch, a)).unwrap().unwrap();
            assert!(ciborium::from_reader::<Vec<AccountUpdateRecord>, _>(v.as_ref()).is_err());
        }

        let report = migrate_ol_db(&db).unwrap();
        assert!(report.ran);
        assert_eq!(report.account_update_rows_migrated, 3);
        assert_eq!(report.account_update_records_migrated, 5);

        let decode = |epoch: Epoch| -> Vec<AccountUpdateRecord> {
            let v = tree.get(key_bytes(epoch, a)).unwrap().unwrap();
            ciborium::from_reader(v.as_ref()).expect("current decode")
        };

        // expected prev_next_inbox_idx per record:
        //   e0: [0, 2]   (chain from prior terminal 0, then 2)
        //   e1: [5]      (terminal of e0 was 5)
        //   e2: [9, 9]   (terminal of e1 was 9, then 9)
        let e0 = decode(0);
        assert_eq!(
            e0.iter().map(|r| r.prev_next_inbox_idx()).collect::<Vec<_>>(),
            vec![0, 2]
        );
        assert_eq!(
            e0.iter().map(|r| r.next_inbox_idx()).collect::<Vec<_>>(),
            vec![2, 5]
        );
        let e1 = decode(1);
        assert_eq!(e1[0].prev_next_inbox_idx(), 5);
        assert_eq!(e1[0].next_inbox_idx(), 9);
        let e2 = decode(2);
        assert_eq!(
            e2.iter().map(|r| r.prev_next_inbox_idx()).collect::<Vec<_>>(),
            vec![9, 9]
        );
    }

    /// Two accounts must chain independently (no cross-account leakage).
    #[test]
    fn migrate_ol_db_isolates_accounts() {
        let db = temp_db();
        let tree = db.open_tree(ACCOUNT_UPDATE_TREE).unwrap();
        let a = acct(1);
        let b = acct(2);

        tree.insert(key_bytes(0, a), v0_row(&[3, 8])).unwrap();
        tree.insert(key_bytes(0, b), v0_row(&[100])).unwrap();
        tree.insert(key_bytes(1, b), v0_row(&[105])).unwrap();

        migrate_ol_db(&db).unwrap();

        let decode = |epoch: Epoch, account: AccountId| -> Vec<AccountUpdateRecord> {
            let v = tree.get(key_bytes(epoch, account)).unwrap().unwrap();
            ciborium::from_reader(v.as_ref()).unwrap()
        };
        // account a, epoch 0: prevs [0, 3]
        let a0 = decode(0, a);
        assert_eq!(
            a0.iter().map(|r| r.prev_next_inbox_idx()).collect::<Vec<_>>(),
            vec![0, 3]
        );
        // account b: epoch 0 prev [0], epoch 1 prev [100]
        assert_eq!(decode(0, b)[0].prev_next_inbox_idx(), 0);
        assert_eq!(decode(1, b)[0].prev_next_inbox_idx(), 100);
    }

    /// Idempotency: a second run is a no-op and leaves bytes unchanged.
    #[test]
    fn migrate_ol_db_is_idempotent() {
        let db = temp_db();
        let tree = db.open_tree(ACCOUNT_UPDATE_TREE).unwrap();
        let a = acct(7);
        tree.insert(key_bytes(0, a), v0_row(&[4, 6])).unwrap();

        let r1 = migrate_ol_db(&db).unwrap();
        assert!(r1.ran);
        assert_eq!(r1.account_update_rows_migrated, 1);
        let after_first = tree.get(key_bytes(0, a)).unwrap().unwrap();

        let r2 = migrate_ol_db(&db).unwrap();
        assert!(!r2.ran);
        assert_eq!(r2.account_update_rows_migrated, 0);
        let after_second = tree.get(key_bytes(0, a)).unwrap().unwrap();

        assert_eq!(after_first, after_second);
    }

    /// In-vec chaining: each record's `prev_next_inbox_idx` is the previous
    /// record's `next_inbox_idx`; first record chains from `prev_terminal`.
    #[test]
    fn in_vec_chaining() {
        // next_inbox_idx values within a single epoch row.
        let next_idxs = [3u64, 7, 12];
        let prev_terminal_start = 0u64;

        let mut prev_terminal = prev_terminal_start;
        let mut prevs = Vec::new();
        for n in next_idxs {
            prevs.push(prev_terminal);
            prev_terminal = n;
        }
        assert_eq!(prevs, vec![0, 3, 7]);
        assert_eq!(prev_terminal, 12);
    }

    /// Cross-epoch chaining: the first record of epoch N+1 chains from the
    /// terminal `next_inbox_idx` of epoch N.
    #[test]
    fn cross_epoch_chaining() {
        // epoch 0 row: next=[2,5]; epoch 1 row: next=[9]; epoch 2 row: next=[9,11]
        let epochs: Vec<Vec<u64>> = vec![vec![2, 5], vec![9], vec![9, 11]];
        let mut prev_terminal = 0u64;
        let mut all_prevs: Vec<Vec<u64>> = Vec::new();
        for row in &epochs {
            let mut prevs = Vec::new();
            for &n in row {
                prevs.push(prev_terminal);
                prev_terminal = n;
            }
            all_prevs.push(prevs);
        }
        assert_eq!(all_prevs, vec![vec![0, 2], vec![5], vec![9, 9]]);
        assert_eq!(prev_terminal, 11);
    }
}
