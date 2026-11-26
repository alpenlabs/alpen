//! Prepared by the current node using the actual database backend.
use std::{collections::BTreeMap, env, fs, path::PathBuf};

use serde_json::Value;
use strata_db_store_sled::{
    SLED_NAME, SledDbConfig, init_core_dbs, open_sled_database_with_handle,
};
use strata_db_types::{backend::DatabaseBackend, ol_block::OLBlockDatabase};
use strata_identifiers::EpochCommitment;

use super::make_block;

#[test]
#[ignore = "creates a fresh current-node destination for the offline importer"]
fn prepare_promoted_destination() {
    let root = PathBuf::from(env::var_os("STRATA_V0_FIXTURE_OUT").unwrap());
    let rows: Vec<Value> =
        serde_json::from_slice(&fs::read(root.join("responses.json")).unwrap()).unwrap();
    let terminal = serde_json::from_value(rows[2]["header"]["blkid"].clone()).unwrap();
    let base = EpochCommitment::new(1, 2, terminal);
    let path = root.join("promoted");
    fs::create_dir(&path).unwrap();
    let (typed, raw) = open_sled_database_with_handle(&path, SLED_NAME).unwrap();
    let backend = init_core_dbs(typed, SledDbConfig::test()).unwrap();
    let db = backend.ol_block_db();
    db.promote_to_history_anchor(base).unwrap();
    let first = make_block(3, 2, *base.last_blkid());
    let second = make_block(4, 2, first.header().compute_blkid());
    let ids = vec![
        first.header().compute_blkid(),
        second.header().compute_blkid(),
    ];
    db.put_block_data(first).unwrap();
    db.put_block_data(second).unwrap();
    db.replace_canonical_suffix_from(3, ids).unwrap();
    let mut trees = BTreeMap::new();
    for name in raw.tree_names() {
        let entries: Vec<_> = raw
            .open_tree(&name)
            .unwrap()
            .iter()
            .map(|row| {
                let (key, value) = row.unwrap();
                (hex::encode(key), hex::encode(value))
            })
            .collect();
        trees.insert(String::from_utf8(name.to_vec()).unwrap(), entries);
    }
    let expected: Value =
        serde_json::from_slice(&fs::read(env::var_os("STRATA_PROMOTED_FIXTURE").unwrap()).unwrap())
            .unwrap();
    assert_eq!(serde_json::to_value(trees).unwrap(), expected);
}
