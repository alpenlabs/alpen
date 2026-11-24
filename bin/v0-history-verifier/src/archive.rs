//! Publishes a complete, indexed V0 archive after the verified walk succeeds.

use std::{
    fs::{self, File},
    io,
    path::Path,
};

use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, params};
use serde::Serialize;
use sha2::{Digest, Sha256};
use strata_db_store_sled::SLED_NAME;
use strata_identifiers::{Epoch, Slot};
use strata_ol_rpc_types::RpcBlockHeaderEntry;

use crate::{
    Args,
    walk::{self, Counts, Report},
};

const DATABASE: &str = "history.sqlite3";
const DECODER_REVISION: &str = "048c5bf88bd02e19d60d508e0f9f3e1b13a24510";

#[derive(Serialize)]
struct Manifest<'a> {
    format_version: u32,
    source_snapshot: &'a str,
    decoder_revision: &'static str,
    first_slot: Slot,
    genesis_id: String,
    terminal_slot: Slot,
    terminal_epoch: Epoch,
    terminal_id: String,
    counts: &'a Counts,
    database: &'static str,
    database_sha256: String,
}

pub(super) fn export(args: &Args, destination: &Path, source: &str) -> Result<Report> {
    ensure!(
        !destination.try_exists()?,
        "archive destination already exists: {}",
        destination.display()
    );
    let parent = destination
        .parent()
        .context("missing archive parent directory")?;
    let parent = if parent.as_os_str().is_empty() {
        Path::new(".")
    } else {
        parent
    };
    let source_db = args.datadir.join("sled").join(SLED_NAME).canonicalize()?;
    ensure!(
        !parent.canonicalize()?.starts_with(source_db),
        "archive destination is inside the source database"
    );
    let temp = tempfile::Builder::new()
        .prefix(".v0-export-")
        .tempdir_in(parent)?;
    let path = temp.path().join(DATABASE);
    let report = write_database(args, &path)?;
    let mut database = File::open(&path)?;
    database.sync_all()?;
    let mut hash = Sha256::new();
    io::copy(&mut database, &mut hash)?;
    let manifest = Manifest {
        format_version: 1,
        source_snapshot: source,
        decoder_revision: DECODER_REVISION,
        first_slot: 0,
        genesis_id: hex::encode(args.genesis_id.as_ref()),
        terminal_slot: args.terminal_slot,
        terminal_epoch: args.terminal_epoch,
        terminal_id: hex::encode(args.terminal_id.as_ref()),
        counts: &report.counts,
        database: DATABASE,
        database_sha256: hex::encode(hash.finalize()),
    };
    let file = File::create(temp.path().join("manifest.json"))?;
    serde_json::to_writer_pretty(&file, &manifest)?;
    file.sync_all()?;
    File::open(temp.path())?.sync_all()?;
    // A completed destination is nonempty, so a racing publisher cannot replace it.
    fs::rename(temp.path(), destination).context("publishing verified archive")?;
    let _ = temp.keep();
    File::open(parent)?.sync_all()?;
    Ok(report)
}

fn write_database(args: &Args, path: &Path) -> Result<Report> {
    let mut db = Connection::open(path)?;
    let tx = db.transaction()?;
    tx.execute_batch(
        "CREATE TABLE blocks (
        slot INTEGER PRIMARY KEY CHECK (slot >= 0),
        block_id BLOB NOT NULL UNIQUE CHECK (length(block_id) = 32),
        epoch INTEGER NOT NULL,
        raw_block BLOB NOT NULL,
        header TEXT NOT NULL,
        transactions TEXT NOT NULL,
        account_changes TEXT,
        CHECK ((slot = 0) = (account_changes IS NULL))
    )",
    )?;
    let mut insert = tx.prepare(
        "INSERT INTO blocks
        (slot, block_id, epoch, raw_block, header, transactions, account_changes)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?;
    let report = walk::walk(args, |row| {
        let header = RpcBlockHeaderEntry::from(row.block);
        insert.execute(params![
            header.slot(),
            header.blkid().as_ref().as_slice(),
            header.epoch(),
            row.raw,
            serde_json::to_string(&header)?,
            serde_json::to_string(&row.transactions)?,
            row.account_changes
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?
        ])?;
        Ok(())
    })?;
    drop(insert);
    tx.commit()?;
    db.close().map_err(|(_, error)| error)?;
    Ok(report)
}
