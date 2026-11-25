//! The three-slot fixture is the output of the v0.3.5 importer's offline test chain.
//! Slot 1 has a snark update, a message, a created account and an updated account.
use sha2::{Digest, Sha256};
use strata_db_store_sled::ol::schemas::v0_history::{
    BLOCKS, CHANGES, COMPLETE_KEY, META, TXS, digest,
};

use super::*;
use crate::rpc::history::HistoryArchive;

fn fixture_db() -> sled::Db {
    let db = sled::Config::new().temporary(true).open().unwrap();
    let rows: Vec<(String, String, String)> =
        serde_json::from_str(include_str!("../../../testdata/v0-history/rows.json")).unwrap();
    for (tree, key, value) in rows {
        db.open_tree(tree)
            .unwrap()
            .insert(hex::decode(key).unwrap(), hex::decode(value).unwrap())
            .unwrap();
    }
    db
}

fn fixture_anchors() -> (OLBlockId, EpochCommitment) {
    let genesis = serde_json::from_value(json!(
        "7ff1c08429688b0ff921eafb75898c10ba8ade25ffd5317cb3ba0c3a414a7cd1"
    ))
    .unwrap();
    let last = serde_json::from_value(json!(
        "080b6a9d69059feb4e6aa132ed168719c166046c67424a60aa647c993fbbcf1b"
    ))
    .unwrap();
    (genesis, EpochCommitment::new(1, 2, last))
}

fn fixture_archive() -> HistoryArchive {
    let (genesis, base) = fixture_anchors();
    HistoryArchive::discover(&fixture_db(), || Ok((genesis, base)))
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn explorer_reads_cross_the_linked_v0_v1_boundary() {
    let archive = fixture_archive();
    let base = archive.terminal();
    let first = make_block(3, 2, *base.last_blkid());
    let second = make_block(4, 2, first.header().compute_blkid());
    let provider = MockProvider::new()
        .with_history_base(base)
        .with_block_and_state(&first, genesis_ol_state())
        .with_block_and_state(&second, genesis_ol_state())
        .with_write_batch(
            first.header().compute_block_commitment(),
            WriteBatch::default(),
        );
    let rpc = make_rpc(provider).with_history_archive(Some(archive));
    let headers = rpc.get_headers_in_range(0, 4).await.unwrap();
    let blocks = rpc.get_raw_blocks_range(0, 4).await.unwrap();
    assert_eq!(headers.len(), 5);
    assert_eq!(blocks.len(), 5);
    for (slot, (header, block)) in headers.iter().zip(&blocks).enumerate() {
        assert_eq!((header.slot(), block.slot()), (slot as u64, slot as u64));
        assert_eq!(
            (header.epoch(), header.blkid()),
            (block.epoch(), block.blkid())
        );
        if slot > 0 {
            assert_eq!(header.parent_blkid(), headers[slot - 1].blkid());
        }
        assert_eq!(
            rpc.get_raw_block_by_id(header.blkid()).await.unwrap().0,
            block.raw_block().0
        );
    }
    assert_eq!(headers[2].blkid(), *base.last_blkid());
    assert_eq!(blocks[3].raw_block().0, first.as_ssz_bytes());
    assert_eq!(
        format!("{:x}", Sha256::digest(&blocks[1].raw_block().0)),
        "5eb2d973c0049872bb8eef56536a31ad68d7eab0bd9ac1eeea8ff30835ec28da"
    );
    let txs = json!(rpc.get_block_transactions(1).await.unwrap());
    assert_eq!(txs.as_array().unwrap().len(), 2);
    assert_eq!(
        txs[0]["txid"],
        "38f11b0f43ae6683699ce00bea40b94a4cd9ea2a6c0088ef170a98724eb4c6cd"
    );
    assert_eq!(
        txs[1]["txid"],
        "2678efaa72b33d62d61a45b933d297ca4b69cbc39b67ccefa3764eb50ae5aa2c"
    );
    assert!(txs[0].get("new_predicate").is_none());
    let changes = json!(rpc.get_block_account_changes(1).await.unwrap());
    assert_eq!(changes["changes"][0]["change_type"], "created");
    assert_eq!(changes["changes"][0]["state"]["balance_sats"], 10);
    assert_eq!(changes["changes"][1]["change_type"], "updated");
    assert_eq!(changes["changes"][1]["state"]["balance_sats"], 20);
    assert_eq!(
        changes["changes"][1]["state"]["update_vk"],
        json!("AlwaysAccept")
    );
    assert!(rpc.get_block_transactions(0).await.unwrap().is_empty());
    let err = rpc.get_block_account_changes(0).await.unwrap_err();
    assert_eq!(
        (err.code(), err.message()),
        (
            INVALID_PARAMS_CODE,
            "No OL write batch found for block 0@7ff1c0..4a7cd1"
        )
    );
    for slot in [2, 3] {
        assert!(
            rpc.get_block_account_changes(slot)
                .await
                .unwrap()
                .changes()
                .is_empty()
        );
        assert!(rpc.get_block_transactions(slot).await.unwrap().is_empty());
    }
    assert_eq!(
        rpc.get_raw_block_by_id(null_blkid())
            .await
            .unwrap_err()
            .code(),
        INVALID_PARAMS_CODE
    );
    assert_eq!(
        rpc.get_headers_in_range(0, u64::MAX)
            .await
            .unwrap_err()
            .code(),
        INVALID_PARAMS_CODE
    );
    assert_eq!(
        rpc.get_raw_blocks_range(0, u64::MAX)
            .await
            .unwrap_err()
            .code(),
        INVALID_PARAMS_CODE
    );
}

#[tokio::test]
async fn mixed_ranges_reject_a_live_fork_at_cutover() {
    let archive = fixture_archive();
    let first = make_block(3, 2, null_blkid());
    let rpc = make_rpc(
        MockProvider::new()
            .with_history_base(archive.terminal())
            .with_block_and_state(&first, genesis_ol_state()),
    )
    .with_history_archive(Some(archive));
    for error in [
        rpc.get_headers_in_range(2, 3).await.unwrap_err(),
        rpc.get_raw_blocks_range(2, 3).await.unwrap_err(),
    ] {
        assert_eq!(error.code(), INTERNAL_ERROR_CODE);
        assert_eq!(
            error.message(),
            "first live block does not descend from the archived terminal"
        );
    }
}

#[test]
fn archive_rehashes_each_header_even_with_a_matching_checksum() {
    let db = fixture_db();
    let headers = db.open_tree("V0OLHeaderBySlot").unwrap();
    let key = 1u64.to_be_bytes();
    let mut bytes = headers.get(key).unwrap().unwrap().to_vec();
    bytes[0] ^= 1; // Alter slot one's timestamp while preserving its slot.
    headers.insert(key, bytes).unwrap();
    update_digest(&db);
    let error = HistoryArchive::discover(&db, || Ok(fixture_anchors())).unwrap_err();
    assert_eq!(error.to_string(), "V0 header hash mismatch at slot 1");
}

#[test]
fn archive_requires_completion_intact_rows_and_matching_anchors() {
    let (genesis, base) = fixture_anchors();
    let db = fixture_db();
    for anchors in [
        (null_blkid(), base),
        (genesis, EpochCommitment::new(0, 2, *base.last_blkid())),
        (genesis, EpochCommitment::new(1, 2, null_blkid())),
    ] {
        assert!(HistoryArchive::discover(&db, || Ok(anchors)).is_err());
    }
    let batches = db.open_tree("V0OLWriteBatchById").unwrap();
    batches.remove(base.last_blkid().as_ref()).unwrap();
    assert!(
        HistoryArchive::discover(&db, || Ok((genesis, base)))
            .unwrap_err()
            .to_string()
            .contains("wrong row count")
    );
    batches
        .insert(base.last_blkid().as_ref(), b"corrupt")
        .unwrap();
    assert!(
        HistoryArchive::discover(&db, || Ok((genesis, base)))
            .unwrap_err()
            .to_string()
            .contains("checksum mismatch")
    );
    db.open_tree("V0OLArchiveMeta")
        .unwrap()
        .remove(b"complete")
        .unwrap();
    assert!(
        HistoryArchive::discover(&db, || panic!("unexpected anchor lookup"))
            .unwrap()
            .is_none()
    );
    let empty = sled::Config::new().temporary(true).open().unwrap();
    let names = empty.tree_names();
    assert!(
        HistoryArchive::discover(&empty, || panic!("unexpected anchor lookup"))
            .unwrap()
            .is_none()
    );
    assert_eq!(empty.tree_names(), names);
}

#[tokio::test]
async fn explorer_status_checkpoint_and_manifest_stay_on_the_live_provider() {
    fn provider() -> MockProvider {
        spec_status_provider(OLSpecId::V1, Some(make_empty_manifest(1, 0)), true).0
    }
    let live = make_rpc(provider());
    let enabled = make_rpc(provider()).with_history_archive(Some(fixture_archive()));
    assert_eq!(
        json!(enabled.chain_status().await.unwrap()),
        json!(live.chain_status().await.unwrap())
    );
    assert_eq!(
        json!(enabled.get_checkpoint_info(42).await.unwrap()),
        json!(live.get_checkpoint_info(42).await.unwrap())
    );
    let manifest = live.get_asm_manifest_commitment(1).await.unwrap().unwrap();
    assert_eq!(
        enabled
            .get_asm_manifest_commitment(1)
            .await
            .unwrap()
            .unwrap()
            .0,
        manifest.0
    );
}

fn update_digest(db: &sled::Db) {
    let meta = db.open_tree(META).unwrap();
    let mut record: Value =
        serde_json::from_slice(&meta.get(COMPLETE_KEY).unwrap().unwrap()).unwrap();
    record["content_sha256"] = json!(digest(db).unwrap());
    meta.insert(COMPLETE_KEY, serde_json::to_vec(&record).unwrap())
        .unwrap();
}

#[test]
fn verified_digest_cannot_hide_invalid_served_rows() {
    let (_, base) = fixture_anchors();
    let id = base.last_blkid().as_ref();
    let check = |name: &str, value: Vec<u8>, error: &str| {
        let db = fixture_db();
        db.open_tree(name).unwrap().insert(id, value).unwrap();
        update_digest(&db);
        let err = HistoryArchive::discover(&db, || Ok(fixture_anchors())).unwrap_err();
        assert!(err.to_string().contains(error), "{err:#}");
    };
    let db = fixture_db();
    let mut bytes = db
        .open_tree(BLOCKS)
        .unwrap()
        .get(id)
        .unwrap()
        .unwrap()
        .to_vec();
    *bytes.last_mut().unwrap() ^= 1;
    check(BLOCKS, bytes, "V0 body hash mismatch");
    let genesis = fixture_anchors().0;
    let other = db
        .open_tree(BLOCKS)
        .unwrap()
        .get(genesis.as_ref())
        .unwrap()
        .unwrap();
    check(BLOCKS, other.to_vec(), "signed block header differs");
    check(TXS, b"{}".to_vec(), "invalid transactions");
    check(CHANGES, b"[]".to_vec(), "invalid account changes");
    let bytes = db.open_tree(CHANGES).unwrap().get(id).unwrap().unwrap();
    let mut changes: Value = serde_json::from_slice(&bytes).unwrap();
    changes["blkid"] = json!(genesis);
    check(
        CHANGES,
        serde_json::to_vec(&changes).unwrap(),
        "account changes reference a different block",
    );
}

#[tokio::test]
async fn archive_ranges_include_the_terminal_once_at_each_endpoint_and_limit() {
    let archive = fixture_archive();
    let base = archive.terminal();
    let mut parent = *base.last_blkid();
    let mut provider = MockProvider::new().with_history_base(base);
    let state = genesis_ol_state();
    for slot in 3..5000 {
        let block = make_block(slot, 2, parent);
        parent = block.header().compute_blkid();
        provider = provider.with_block_and_state(&block, state.clone());
    }
    let rpc = make_rpc(provider).with_history_archive(Some(archive));
    for (start, end) in [(0, 2), (2, 2), (3, 4), (0, 4999)] {
        let expected: Vec<_> = (start..=end).collect();
        let headers = rpc.get_headers_in_range(start, end).await.unwrap();
        let blocks = rpc.get_raw_blocks_range(start, end).await.unwrap();
        assert_eq!(
            headers.iter().map(|row| row.slot()).collect::<Vec<_>>(),
            expected
        );
        assert_eq!(
            blocks.iter().map(|row| row.slot()).collect::<Vec<_>>(),
            expected
        );
    }
    assert!(rpc.get_headers_in_range(2, 5000).await.is_err());
    assert!(rpc.get_raw_blocks_range(2, 5000).await.is_err());
}
