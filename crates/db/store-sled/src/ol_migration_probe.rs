//! Test-only empirical probe for the post-#1849 OL sled CBOR layout break.
//!
//! Confirms, against a *real* staging OL sled DB, that the #1942
//! `AccountUpdateRecord::prev_next_inbox_idx` insertion is a decode break:
//! the current struct fails to decode old `OLAccountUpdateEntrySchema` rows,
//! while a V0 mirror (without the field) succeeds. Run on k2 against a copy:
//!
//! ```text
//! STAGING_OL_SLED=/Users/k2/staging-v2-dbs/ol-sled/sled/strata-client \
//!   cargo test -p strata-db-store-sled --lib ol_migration_probe -- --nocapture
//! ```
//!
//! No-ops (passes) when `STAGING_OL_SLED` is unset, so it is inert in CI.

use std::env;

use serde::{Deserialize, Serialize};
use strata_db_types::ol_state_index::AccountUpdateRecord;
use strata_identifiers::{Hash, OLBlockCommitment};

/// Mirror of `AccountUpdateMeta` (unchanged by #1942).
#[derive(Clone, Debug, Serialize, Deserialize)]
struct AccountUpdateMetaV0 {
    block_commitment: Option<OLBlockCommitment>,
    new_state_root: Hash,
}

/// V0 (pre-#1942) `AccountUpdateRecord`: no `prev_next_inbox_idx`.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct AccountUpdateRecordV0 {
    update_meta: Option<AccountUpdateMetaV0>,
    seq_no: u64,
    next_inbox_idx: u64,
    extra_data: Option<Vec<u8>>,
}

#[test]
fn ol_migration_probe_layout_break() {
    let Ok(path) = env::var("STAGING_OL_SLED") else {
        eprintln!("STAGING_OL_SLED unset; skipping OL probe");
        return;
    };
    let db = sled::open(&path).expect("open staging OL sled");
    let trees: Vec<String> = db
        .tree_names()
        .iter()
        .map(|t| String::from_utf8_lossy(t).into_owned())
        .collect();
    eprintln!("OL trees: {trees:?}");

    let tree = db
        .open_tree("OLAccountUpdateEntrySchema")
        .expect("open OLAccountUpdateEntrySchema");

    let (mut rows, mut total_records) = (0u64, 0u64);
    let (mut new_ok, mut v0_ok) = (0u64, 0u64);
    for kv in tree.iter() {
        let (_k, v) = kv.expect("iter");
        rows += 1;
        // Current struct (Vec<AccountUpdateRecord>).
        if let Ok(recs) = ciborium::from_reader::<Vec<AccountUpdateRecord>, _>(v.as_ref()) {
            new_ok += 1;
            total_records += recs.len() as u64;
        }
        if ciborium::from_reader::<Vec<AccountUpdateRecordV0>, _>(v.as_ref()).is_ok() {
            v0_ok += 1;
        }
    }
    eprintln!(
        "OL ACCOUNT_UPDATE: rows={rows} new_decode_ok={new_ok} v0_decode_ok={v0_ok} total_records(new)={total_records}"
    );

    assert!(rows > 0, "no OLAccountUpdateEntrySchema rows in staging DB");
    assert_eq!(new_ok, 0, "current AccountUpdateRecord unexpectedly decoded old rows");
    assert_eq!(v0_ok, rows, "V0 layout failed to decode some old rows");
}

/// Post-migration inverse check: after running the migration on a copy, EVERY
/// `OLAccountUpdateEntrySchema` row must decode with the CURRENT struct AND have
/// a correctly-chained `prev_next_inbox_idx` within each row. Point
/// `MIGRATED_OL_SLED` at the migrated copy.
#[test]
fn ol_migration_probe_post_migration_decode() {
    let Ok(path) = env::var("MIGRATED_OL_SLED") else {
        eprintln!("MIGRATED_OL_SLED unset; skipping post-migration OL decode check");
        return;
    };
    let db = sled::open(&path).expect("open migrated OL sled");
    let tree = db
        .open_tree("OLAccountUpdateEntrySchema")
        .expect("open OLAccountUpdateEntrySchema");

    let (mut rows, mut ok, mut records, mut chain_ok) = (0u64, 0u64, 0u64, 0u64);
    for kv in tree.iter() {
        let (_k, v) = kv.expect("iter");
        rows += 1;
        if let Ok(recs) = ciborium::from_reader::<Vec<AccountUpdateRecord>, _>(v.as_ref()) {
            ok += 1;
            records += recs.len() as u64;
            // In-row chaining invariant: record[i].prev == record[i-1].next.
            let mut row_ok = true;
            for w in recs.windows(2) {
                if w[1].prev_next_inbox_idx() != w[0].next_inbox_idx() {
                    row_ok = false;
                    break;
                }
            }
            if row_ok {
                chain_ok += 1;
            }
        }
    }
    eprintln!(
        "POST-MIGRATION OL: account_update {ok}/{rows} records={records} in_row_chain_ok={chain_ok}/{rows}"
    );
    assert_eq!(ok, rows, "some OL rows fail current decode post-migration");
    assert_eq!(chain_ok, rows, "some OL rows have broken in-row prev/next chaining");
}
