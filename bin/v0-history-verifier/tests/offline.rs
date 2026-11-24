//! Exercise the verifier CLI against records persisted with the release's codecs.

use std::{
    fs::{self, OpenOptions},
    path::Path,
    process::{Command, Output},
};

use anyhow as _;
use argh as _;
use rusqlite::{Connection, OpenFlags};
use serde as _;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use ssz::Decode;
use strata_db_store_sled::{SLED_NAME, SledDbConfig, open_sled_backend};
use strata_db_types::{
    backend::DatabaseBackend, ol_block::OLBlockDatabase, ol_state::OLStateDatabase,
};
use strata_identifiers::{AccountId, Buf32, Buf64, OLBlockCommitment, OLBlockId, Slot};
use strata_ol_chain_types::{
    BlockFlags, OLBlock, OLBlockBody, OLBlockHeader, OLTransaction, OLTransactionData, OLTxSegment,
    SauTxLedgerRefs, SauTxOperationData, SauTxPayload, SauTxProofState, SauTxUpdateData,
    SignedOLBlockHeader, TransactionPayload, TxProofs,
};
use strata_ol_rpc_types::{
    RpcAccountChangeType, RpcBlockAccountChanges, RpcBlockEntry, RpcBlockHeaderEntry, RpcOLTxDetail,
};
use strata_ol_state_types::{OLAccountState, OLAccountTypeState, WriteBatch};
use tempfile::{TempDir, tempdir};
use typed_sled as _;

fn block(slot: Slot, parent: OLBlockId, txs: Vec<OLTransaction>, terminal: bool) -> OLBlock {
    let body = OLBlockBody::new_common(OLTxSegment::new(txs).unwrap());
    let mut flags = BlockFlags::zero();
    flags.set_is_terminal(terminal);
    let header = OLBlockHeader::new(
        slot,
        flags,
        slot,
        u32::from(slot > 0),
        parent,
        body.compute_hash_commitment(),
        Buf32::zero(),
        Buf32::zero(),
    );
    let signature = if slot == 0 {
        Buf64::zero()
    } else {
        Buf64::from([0xa5; 64])
    };
    OLBlock::new(SignedOLBlockHeader::new(header, signature), body)
}

struct Fixture {
    dir: TempDir,
    genesis: OLBlockId,
    middle: OLBlockId,
    terminal: OLBlockId,
    txids_digest: String,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempdir().unwrap();
        let db = open_sled_backend(dir.path(), SLED_NAME, SledDbConfig::test()).unwrap();
        let target = AccountId::new([1; 32]);
        let update = SauTxOperationData::new(
            SauTxUpdateData::new(1, SauTxProofState::new(0, Buf32::zero()), vec![7]),
            vec![],
            SauTxLedgerRefs::new_empty(),
        );
        let txs = vec![
            OLTransaction::new(
                OLTransactionData::new(
                    TransactionPayload::SnarkAccountUpdate(SauTxPayload::new(target, update)),
                    Default::default(),
                ),
                TxProofs::new_empty(),
            ),
            OLTransaction::new(
                OLTransactionData::from_gam_bytes(target, vec![4]).unwrap(),
                TxProofs::new_empty(),
            ),
        ];
        let mut digest = Sha256::new();
        for tx in &txs {
            digest.update(tx.compute_txid().as_ref());
        }
        let genesis = block(0, OLBlockId::null(), vec![], true);
        let genesis_id = genesis.header().compute_blkid();
        let middle = block(1, genesis_id, txs, false);
        let middle_id = middle.header().compute_blkid();
        let sibling = block(1, genesis_id, vec![], false);
        let terminal = block(2, middle_id, vec![], true);
        let terminal_id = terminal.header().compute_blkid();

        let mut batch = WriteBatch::default();
        let serial = 42u32.into();
        let state =
            OLAccountState::new(serial, 10u64.try_into().unwrap(), OLAccountTypeState::Empty);
        batch.ledger_mut().create_account_raw(target, state, serial);
        let updated = OLAccountState::new(
            7u32.into(),
            20u64.try_into().unwrap(),
            OLAccountTypeState::Empty,
        );
        batch
            .ledger_mut()
            .update_account(AccountId::new([2; 32]), updated);
        for (block, batch) in [
            (&middle, batch),
            (&sibling, WriteBatch::default()),
            (&terminal, WriteBatch::default()),
        ] {
            db.ol_state_db()
                .put_ol_write_batch(block.header().compute_block_commitment(), batch)
                .unwrap();
        }
        for block in [genesis, sibling, middle, terminal] {
            db.ol_block_db().put_block_data(block).unwrap();
        }
        Self {
            dir,
            genesis: genesis_id,
            middle: middle_id,
            terminal: terminal_id,
            txids_digest: hex::encode(digest.finalize()),
        }
    }

    fn run(&self) -> Output {
        run(self.dir.path(), self.genesis, self.terminal, 2, 1)
    }

    fn export(&self, out: &Path) -> Output {
        command(self.dir.path(), self.genesis, self.terminal, 2, 1)
            .arg("--archive-dir")
            .arg(out)
            .args(["--source-snapshot", "local-fixture"])
            .output()
            .unwrap()
    }

    fn raw_db(&self) -> sled::Db {
        wait_for_sled_close(self.dir.path());
        sled::open(self.dir.path().join("sled").join(SLED_NAME)).unwrap()
    }
}

// Sled's background I/O can briefly hold the file lock after the last handle drops.
// Each fixture has a private datadir, so the next opener has no competing writer.
fn wait_for_sled_close(datadir: &Path) {
    let db_file = datadir.join("sled").join(SLED_NAME).join("db");
    if db_file.exists() {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(db_file)
            .unwrap();
        file.lock().unwrap();
    }
}

fn run(path: &Path, genesis: OLBlockId, terminal: OLBlockId, slot: Slot, epoch: u32) -> Output {
    command(path, genesis, terminal, slot, epoch)
        .output()
        .unwrap()
}

fn command(
    path: &Path,
    genesis: OLBlockId,
    terminal: OLBlockId,
    slot: Slot,
    epoch: u32,
) -> Command {
    wait_for_sled_close(path);
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_strata-v0-history-verifier"));
    cmd.arg("--datadir").arg(path).args([
        "--genesis-id",
        &hex::encode(genesis.as_ref()),
        "--terminal-id",
        &hex::encode(terminal.as_ref()),
        "--terminal-slot",
        &slot.to_string(),
        "--terminal-epoch",
        &epoch.to_string(),
    ]);
    cmd
}

fn assert_failure(output: Output, slot: Slot, id: OLBlockId, check: &str) {
    let err = String::from_utf8(output.stderr).unwrap();
    assert_eq!(output.status.code(), Some(1), "{err}");
    assert!(output.stdout.is_empty());
    let expected = format!("slot {slot} block {}: {check}", hex::encode(id.as_ref()));
    assert!(err.contains(&expected), "expected {expected}, got {err}");
}

#[test]
fn verifies_linked_history_and_ignores_sibling() {
    let f = Fixture::new();
    let output = f.run();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        report["counts"],
        json!({
            "blocks": 3, "write_batches": 2, "transactions": 2, "created_accounts": 1, "updated_accounts": 1
        })
    );
    assert_eq!(report["txids_sha256_tip_to_genesis"], f.txids_digest);
    assert_eq!(report["genesis_id"], hex::encode(f.genesis.as_ref()));
    assert_eq!(report["terminal_id"], hex::encode(f.terminal.as_ref()));
}

#[test]
fn sibling_batch_cannot_replace_missing_parent_batch() {
    let f = Fixture::new();
    wait_for_sled_close(f.dir.path());
    let db = open_sled_backend(f.dir.path(), SLED_NAME, SledDbConfig::test()).unwrap();
    db.ol_state_db()
        .del_ol_write_batch(OLBlockCommitment::new(1, f.middle))
        .unwrap();
    drop(db);
    assert_failure(f.run(), 1, f.middle, "missing write batch");
}

#[test]
fn rejects_genesis_batch() {
    let f = Fixture::new();
    wait_for_sled_close(f.dir.path());
    let db = open_sled_backend(f.dir.path(), SLED_NAME, SledDbConfig::test()).unwrap();
    db.ol_state_db()
        .put_ol_write_batch(OLBlockCommitment::new(0, f.genesis), WriteBatch::default())
        .unwrap();
    drop(db);
    assert_failure(f.run(), 0, f.genesis, "unexpected genesis write batch");
}

#[test]
fn reports_missing_parent_and_corrupt_block() {
    let f = Fixture::new();
    {
        let db = f.raw_db();
        let tree = db.open_tree("OLBlockSchema").unwrap();
        tree.insert(f.middle.as_ref(), &[0xff]).unwrap();
    }
    assert_failure(f.run(), 1, f.middle, "reading V0 block");
    {
        let db = f.raw_db();
        db.open_tree("OLBlockSchema")
            .unwrap()
            .remove(f.middle.as_ref())
            .unwrap();
    }
    assert_failure(f.run(), 1, f.middle, "missing block");
}

#[test]
fn reports_undecodable_batch_at_its_commitment() {
    let f = Fixture::new();
    {
        let db = f.raw_db();
        let tree = db.open_tree("OLWriteBatchSchema").unwrap();
        let (terminal_key, _) = tree.last().unwrap().unwrap();
        tree.insert(terminal_key, &[0xff]).unwrap();
    }
    assert_failure(f.run(), 2, f.terminal, "reading V0 write batch");
}

#[test]
fn rejects_wrong_endpoints_and_slot() {
    let f = Fixture::new();
    let path = f.dir.path();
    assert_failure(
        run(path, f.genesis, f.middle, 1, 1),
        1,
        f.middle,
        "terminal flag absent",
    );
    assert_failure(
        run(path, f.genesis, f.terminal, 2, 10),
        2,
        f.terminal,
        "expected epoch 10",
    );
    assert_failure(
        run(path, f.genesis, f.terminal, 3, 1),
        3,
        f.terminal,
        "header has slot 2",
    );
    assert_failure(
        run(path, f.middle, f.terminal, 2, 1),
        0,
        f.genesis,
        "expected genesis",
    );
}

#[test]
fn rejects_header_key_and_body_commitment_mismatches() {
    let f = Fixture::new();
    {
        let db = f.raw_db();
        let tree = db.open_tree("OLBlockSchema").unwrap();
        // A sibling at the same slot has a valid encoding and a different unsigned header.
        let sibling = block(1, f.genesis, vec![], false);
        let sibling_bytes = tree
            .get(sibling.header().compute_blkid().as_ref())
            .unwrap()
            .unwrap();
        tree.insert(f.middle.as_ref(), sibling_bytes).unwrap();
    }
    assert_failure(f.run(), 1, f.middle, "header block ID mismatch");

    wait_for_sled_close(f.dir.path());
    let db = open_sled_backend(f.dir.path(), SLED_NAME, SledDbConfig::test()).unwrap();
    let b = db
        .ol_block_db()
        .get_block_data(f.terminal)
        .unwrap()
        .unwrap();
    let tx = OLTransaction::new(
        OLTransactionData::from_gam_bytes(AccountId::new([9; 32]), vec![1]).unwrap(),
        TxProofs::new_empty(),
    );
    let tampered = OLBlock::new(
        b.signed_header().clone(),
        OLBlockBody::new_common(OLTxSegment::new(vec![tx]).unwrap()),
    );
    db.ol_block_db().del_block_data(f.terminal).unwrap();
    db.ol_block_db().put_block_data(tampered).unwrap();
    drop(db);
    assert_failure(f.run(), 2, f.terminal, "body root mismatch");
}

#[test]
fn mistyped_datadir_does_not_create_a_database() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("absent");
    let output = run(&path, OLBlockId::null(), OLBlockId::null(), 1, 1);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("missing sled data file"));
    assert!(!path.exists());
}

#[test]
fn exported_archive_serves_the_five_explorer_projections() {
    let f = Fixture::new();
    let parent = tempdir().unwrap();
    let archive = parent.path().join("archive");
    let output = f.export(&archive);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let manifest: Value =
        serde_json::from_slice(&fs::read(archive.join("manifest.json")).unwrap()).unwrap();
    let db_path = archive.join("history.sqlite3");
    assert_eq!(manifest["format_version"], 1);
    assert_eq!(manifest["source_snapshot"], "local-fixture");
    assert_eq!(
        manifest["decoder_revision"],
        "048c5bf88bd02e19d60d508e0f9f3e1b13a24510"
    );
    assert_eq!(manifest["first_slot"], 0);
    assert_eq!(manifest["terminal_slot"], 2);
    assert_eq!(manifest["terminal_epoch"], 1);
    assert_eq!(manifest["genesis_id"], hex::encode(f.genesis.as_ref()));
    assert_eq!(manifest["terminal_id"], hex::encode(f.terminal.as_ref()));
    assert_eq!(manifest["counts"]["blocks"], 3);
    assert_eq!(manifest["counts"]["write_batches"], 2);
    assert_eq!(
        manifest["database_sha256"],
        hex::encode(Sha256::digest(fs::read(&db_path).unwrap()))
    );

    let db = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let original = f.raw_db();
    let source = original.open_tree("OLBlockSchema").unwrap();
    let mut query = db
        .prepare(
            "SELECT slot, epoch, block_id, raw_block, header, transactions, account_changes
        FROM blocks WHERE slot BETWEEN ?1 AND ?2 ORDER BY slot",
        )
        .unwrap();
    let mut rows = query.query([0, 2]).unwrap();
    for (slot, expected_id) in [f.genesis, f.middle, f.terminal].into_iter().enumerate() {
        let row = rows.next().unwrap().unwrap();
        let source_bytes = source.get(expected_id.as_ref()).unwrap().unwrap();
        let block = OLBlock::from_ssz_bytes(&source_bytes).unwrap();
        let raw_entry = RpcBlockEntry::from(&block);
        assert_eq!(row.get::<_, u64>(0).unwrap(), raw_entry.slot());
        assert_eq!(row.get::<_, u32>(1).unwrap(), raw_entry.epoch());
        assert_eq!(row.get::<_, Vec<u8>>(2).unwrap(), expected_id.as_ref());
        let raw: Vec<u8> = row.get(3).unwrap();
        assert_eq!(raw, source_bytes.as_ref());
        assert_eq!(&raw, &raw_entry.raw_block().0);
        let by_id: Vec<u8> = db
            .query_row(
                "SELECT raw_block FROM blocks WHERE block_id = ?1",
                [expected_id.as_ref().as_slice()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(by_id, raw);

        let header: Value = serde_json::from_str(&row.get::<_, String>(4).unwrap()).unwrap();
        assert_eq!(
            header,
            serde_json::to_value(RpcBlockHeaderEntry::from(&block)).unwrap()
        );
        let transactions: Value = serde_json::from_str(&row.get::<_, String>(5).unwrap()).unwrap();
        let expected: Vec<_> = block
            .body()
            .tx_segment()
            .unwrap()
            .txs()
            .iter()
            .map(RpcOLTxDetail::from)
            .collect();
        assert_eq!(transactions, serde_json::to_value(expected).unwrap());

        let account_json: Option<String> = row.get(6).unwrap();
        if slot == 0 {
            assert_eq!(account_json, None);
            continue;
        }
        let accounts: RpcBlockAccountChanges =
            serde_json::from_str(&account_json.unwrap()).unwrap();
        assert_eq!(accounts.slot(), slot as u64);
        assert_eq!(accounts.blkid(), expected_id);
        if slot == 1 {
            let [created, updated] = accounts.changes() else {
                panic!("expected both fixture accounts")
            };
            assert_eq!(created.id().0, [1; 32]);
            assert_eq!(created.change_type(), RpcAccountChangeType::Created);
            assert_eq!(created.state().balance_sats(), 10);
            assert_eq!(created.state().serial(), 42);
            assert_eq!(updated.id().0, [2; 32]);
            assert_eq!(updated.change_type(), RpcAccountChangeType::Updated);
            assert_eq!(updated.state().balance_sats(), 20);
            assert_eq!(updated.state().serial(), 7);
        } else {
            assert!(accounts.changes().is_empty());
        }
    }
    assert!(rows.next().unwrap().is_none());
}

#[test]
fn failed_export_leaves_no_archive_and_retry_preserves_published_output() {
    let f = Fixture::new();
    let parent = tempdir().unwrap();
    let archive = parent.path().join("history");
    wait_for_sled_close(f.dir.path());
    let db = open_sled_backend(f.dir.path(), SLED_NAME, SledDbConfig::test()).unwrap();
    let commitment = OLBlockCommitment::new(1, f.middle);
    let batch = db
        .ol_state_db()
        .get_ol_write_batch(commitment)
        .unwrap()
        .unwrap();
    db.ol_state_db().del_ol_write_batch(commitment).unwrap();
    drop(db);
    assert_failure(f.export(&archive), 1, f.middle, "missing write batch");
    assert_eq!(fs::read_dir(parent.path()).unwrap().count(), 0);

    wait_for_sled_close(f.dir.path());
    let db = open_sled_backend(f.dir.path(), SLED_NAME, SledDbConfig::test()).unwrap();
    db.ol_state_db()
        .put_ol_write_batch(commitment, batch)
        .unwrap();
    drop(db);
    let output = f.export(&archive);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let manifest = fs::read(archive.join("manifest.json")).unwrap();
    let again = f.export(&archive);
    assert!(!again.status.success());
    assert!(String::from_utf8_lossy(&again.stderr).contains("destination already exists"));
    assert_eq!(fs::read(archive.join("manifest.json")).unwrap(), manifest);
}
