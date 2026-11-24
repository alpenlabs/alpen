//! Read-only access to the format 1 archive produced by the V0 exporter.

use std::fs::File;
use std::io;
use std::path::Path;
use std::sync::Arc;

use rusqlite::{Connection, OpenFlags, OptionalExtension, Row, params};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use strata_identifiers::{Buf32, Epoch, EpochCommitment, OLBlockId, Slot};
use strata_ol_rpc_types::{
    RpcBlockAccountChanges, RpcBlockEntry, RpcBlockHeaderEntry, RpcOLTxDetail,
};
use strata_primitives::HexBytes;
use thiserror::Error;
use tokio::sync::Mutex;
use tokio::task;

const DATABASE: &str = "history.sqlite3";

#[derive(Deserialize)]
struct Manifest {
    format_version: u32,
    database: String,
    database_sha256: String,
    first_slot: Slot,
    genesis_id: OLBlockId,
    terminal_slot: Slot,
    terminal_epoch: Epoch,
    terminal_id: OLBlockId,
    counts: Counts,
}

#[derive(Deserialize)]
struct Counts {
    blocks: u64,
    write_batches: u64,
}

/// Failure to open or read an explorer archive.
#[derive(Debug, Error)]
pub enum ArchiveError {
    #[error("archive I/O: {0}")]
    Io(#[from] io::Error),
    #[error("archive database: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("archive JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("archive read task: {0}")]
    Join(#[from] task::JoinError),
    #[error("invalid V0 archive: {0}")]
    Invalid(&'static str),
}

type ArchiveResult<T> = Result<T, ArchiveError>;

/// A verified snapshot of the explorer projections, shared by the five historical RPCs.
#[derive(Debug, Clone)]
pub struct HistoryArchive {
    db: Arc<Mutex<Connection>>,
    genesis: OLBlockId,
    terminal: EpochCommitment,
}

impl HistoryArchive {
    /// Opens a read-only export and verifies its checksum, coverage and network anchors.
    /// The directory must contain the immutable files published together by the exporter.
    pub fn open(path: &Path, genesis: OLBlockId, terminal: EpochCommitment) -> ArchiveResult<Self> {
        let manifest: Manifest = serde_json::from_reader(File::open(path.join("manifest.json"))?)?;
        if manifest.format_version != 1 || manifest.database != DATABASE {
            return Err(ArchiveError::Invalid(
                "unsupported archive format or database",
            ));
        }
        if manifest.first_slot != 0 || manifest.genesis_id != genesis {
            return Err(ArchiveError::Invalid(
                "genesis does not match node parameters",
            ));
        }
        if (
            manifest.terminal_slot,
            manifest.terminal_epoch,
            manifest.terminal_id,
        ) != (
            terminal.last_slot(),
            terminal.epoch(),
            *terminal.last_blkid(),
        ) {
            return Err(ArchiveError::Invalid(
                "terminal does not match node history base",
            ));
        }

        let database = path.join(DATABASE);
        let db = Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        // Pin the snapshot for the reader's lifetime. Format 1 is a standalone SQLite file.
        db.execute_batch("BEGIN")?;
        let journal: String = db.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
        if journal != "delete" {
            return Err(ArchiveError::Invalid(
                "expected a standalone database with delete journaling",
            ));
        }
        let mut digest = Sha256::new();
        io::copy(&mut File::open(database)?, &mut digest)?;
        if format!("{:x}", digest.finalize()) != manifest.database_sha256 {
            return Err(ArchiveError::Invalid("database SHA-256 mismatch"));
        }
        let (first, last, blocks, batches, genesis_batches): (Slot, Slot, u64, u64, u64) = db
            .query_row(
                "SELECT min(slot), max(slot), count(*), count(account_changes),
                count(CASE WHEN slot = 0 THEN account_changes END) FROM blocks",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )?;
        if first != 0
            || last != terminal.last_slot()
            || blocks != last + 1
            || batches != last
            || genesis_batches != 0
            || blocks != manifest.counts.blocks
            || batches != manifest.counts.write_batches
        {
            return Err(ArchiveError::Invalid(
                "block or write-batch coverage mismatch",
            ));
        }
        for (slot, epoch, id) in [
            (0, 0, genesis),
            (last, terminal.epoch(), *terminal.last_blkid()),
        ] {
            let header: String = db.query_row(
                "SELECT header FROM blocks WHERE slot = ?1 AND epoch = ?2 AND block_id = ?3",
                params![slot, epoch, id.as_ref().as_slice()],
                |row| row.get(0),
            )?;
            let header: RpcBlockHeaderEntry = serde_json::from_str(&header)?;
            if (
                header.slot(),
                header.epoch(),
                header.blkid(),
                header.is_terminal(),
            ) != (slot, epoch, id, true)
            {
                return Err(ArchiveError::Invalid("archived anchor header mismatch"));
            }
        }
        Ok(Self {
            db: Arc::new(Mutex::new(db)),
            genesis,
            terminal,
        })
    }

    /// Returns the original genesis ID.
    pub fn genesis(&self) -> OLBlockId {
        self.genesis
    }

    /// Returns the inclusive upper bound of the archive.
    pub fn terminal(&self) -> EpochCommitment {
        self.terminal
    }

    async fn read<T: Send + 'static>(
        &self,
        query: impl FnOnce(&Connection) -> ArchiveResult<T> + Send + 'static,
    ) -> ArchiveResult<T> {
        let db = self.db.clone().lock_owned().await;
        task::spawn_blocking(move || query(&db)).await?
    }

    /// Returns original V0 block bytes and metadata in ascending slot order.
    pub async fn blocks(&self, start: Slot, end: Slot) -> ArchiveResult<Vec<RpcBlockEntry>> {
        self.read(move |db| {
            let sql = "SELECT slot, epoch, block_id, raw_block FROM blocks
                       WHERE slot BETWEEN ?1 AND ?2 ORDER BY slot";
            read_range(db, sql, start, end, |row| {
                let bytes: [u8; 32] = row.get(2)?;
                Ok(RpcBlockEntry::new(
                    row.get(0)?,
                    row.get(1)?,
                    OLBlockId::from(Buf32::from(bytes)),
                    HexBytes(row.get(3)?),
                ))
            })
        })
        .await
    }

    /// Returns decoded V0 headers in ascending slot order.
    pub async fn headers(&self, start: Slot, end: Slot) -> ArchiveResult<Vec<RpcBlockHeaderEntry>> {
        self.read(move |db| {
            read_range(
                db,
                "SELECT slot, header FROM blocks WHERE slot BETWEEN ?1 AND ?2 ORDER BY slot",
                start,
                end,
                |row| Ok(serde_json::from_str(&row.get::<_, String>(1)?)?),
            )
        })
        .await
    }

    /// Looks up the original SSZ bytes by canonical V0 block ID.
    pub async fn raw_block(&self, id: OLBlockId) -> ArchiveResult<Option<HexBytes>> {
        self.read(move |db| {
            Ok(db
                .query_row(
                    "SELECT raw_block FROM blocks WHERE block_id = ?1",
                    [id.as_ref().as_slice()],
                    |row| row.get(0).map(HexBytes),
                )
                .optional()?)
        })
        .await
    }

    /// Returns the ordered V0 transaction projections with their original transaction IDs.
    pub async fn transactions(&self, slot: Slot) -> ArchiveResult<Vec<RpcOLTxDetail>> {
        self.read(move |db| {
            let json: String = db.query_row(
                "SELECT transactions FROM blocks WHERE slot = ?1",
                [slot],
                |row| row.get(0),
            )?;
            Ok(serde_json::from_str(&json)?)
        })
        .await
    }

    /// Reads a non-genesis block's projected write batch. A missing batch is an archive failure.
    pub async fn account_changes(&self, slot: Slot) -> ArchiveResult<RpcBlockAccountChanges> {
        self.read(move |db| {
            let json: String = db.query_row(
                "SELECT account_changes FROM blocks WHERE slot = ?1",
                [slot],
                |row| row.get(0),
            )?;
            Ok(serde_json::from_str(&json)?)
        })
        .await
    }
}

fn read_range<T>(
    db: &Connection,
    sql: &str,
    start: Slot,
    end: Slot,
    decode: impl Fn(&Row<'_>) -> ArchiveResult<T>,
) -> ArchiveResult<Vec<T>> {
    let mut stmt = db.prepare_cached(sql)?;
    let mut rows = stmt.query(params![start, end])?;
    let mut result = Vec::new();
    for expected in start..=end {
        let row = rows
            .next()?
            .ok_or(ArchiveError::Invalid("missing archived block"))?;
        if row.get::<_, Slot>(0)? != expected {
            return Err(ArchiveError::Invalid("gap in archived slots"));
        }
        result.push(decode(row)?);
    }
    Ok(result)
}
