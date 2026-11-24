//! Exercise the verifier CLI against records persisted with the release's codecs.

use std::{
    fs::OpenOptions,
    path::Path,
    process::{Command, Output},
};

use anyhow as _;
use argh as _;
use serde as _;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
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
use strata_ol_state_types::{OLAccountState, OLAccountTypeState, WriteBatch};
use tempfile::{TempDir, tempdir};

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
    OLBlock::new(SignedOLBlockHeader::new(header, Buf64::zero()), body)
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
    wait_for_sled_close(path);
    Command::new(env!("CARGO_BIN_EXE_strata-v0-history-verifier"))
        .arg("--datadir")
        .arg(path)
        .args([
            "--genesis-id",
            &hex::encode(genesis.as_ref()),
            "--terminal-id",
            &hex::encode(terminal.as_ref()),
            "--terminal-slot",
            &slot.to_string(),
            "--terminal-epoch",
            &epoch.to_string(),
        ])
        .output()
        .unwrap()
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
