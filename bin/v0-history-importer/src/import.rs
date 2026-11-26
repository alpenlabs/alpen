//! Imports only canonical V0 history into an offline, promoted node.

use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, ensure};
use ssz::Encode;
use strata_codec::decode_buf_exact;
use strata_db_store_sled::{
    SLED_NAME,
    ol::schemas::v0_history::{
        BATCHES, BLOCKS, CHANGES, COMPLETE_KEY, CompleteRecord, DATA, FORMAT_VERSION, HEADERS,
        META, TXS, digest, expected_rows, header_key,
    },
};
use strata_identifiers::EpochCommitment;

use crate::{
    Source,
    walk::{self, Report, VerifiedBlock},
};

fn rows(
    block: VerifiedBlock<'_>,
    mut row: impl FnMut(&str, &[u8], &[u8]) -> Result<()>,
) -> Result<()> {
    let header = block.block.header();
    let id = header.compute_blkid();
    let key = id.as_ref();
    row(HEADERS, &header_key(header.slot()), &header.as_ssz_bytes())?;
    row(BLOCKS, key, block.raw)?;
    row(TXS, key, &serde_json::to_vec(&block.transactions)?)?;
    if let Some((batch, changes)) = block.batch {
        row(BATCHES, key, batch)?;
        row(CHANGES, key, &serde_json::to_vec(&changes)?)?;
    }
    Ok(())
}

fn open_target(target: &Path, expected: EpochCommitment) -> Result<sled::Db> {
    let path = target.join("sled").join(SLED_NAME);
    ensure!(path.join("db").is_file(), "missing target sled database");
    let db = sled::open(path).context("opening offline target database")?;
    let base_tree = "OLHistoryBaseSchema";
    ensure!(
        db.tree_names()
            .iter()
            .any(|n| n.as_ref() == base_tree.as_bytes()),
        "target has no history base"
    );
    let bytes = db
        .open_tree(base_tree)?
        .get([0])?
        .context("target has no history base")?;
    let base: EpochCommitment = decode_buf_exact(&bytes)?;
    ensure!(
        base == expected,
        "target history base differs: expected {expected:?}, found {base:?}"
    );
    Ok(db)
}

pub(super) fn remove(target: &Path, expected: EpochCommitment) -> Result<()> {
    let db = open_target(target, expected)?;
    if db
        .tree_names()
        .iter()
        .any(|name| name.as_ref() == META.as_bytes())
    {
        db.open_tree(META)?.remove(COMPLETE_KEY)?;
        db.flush()?;
    }
    for name in DATA.into_iter().chain([META]) {
        db.drop_tree(name)?;
    }
    db.flush()?;
    Ok(())
}

pub(super) fn import(args: &Source, target: &Path, source: &str, revision: &str) -> Result<Report> {
    ensure!(
        target.join("sled").join(SLED_NAME).canonicalize()?
            != args.datadir.join("sled").join(SLED_NAME).canonicalize()?,
        "source and target must be different databases"
    );
    let input = walk::open_source(args)?;
    let db = open_target(target, args.terminal.commitment())?;
    import_into(args, source, revision, &input, &db, persist_block)
}

fn persist_block(db: &sled::Db, block: VerifiedBlock<'_>) -> Result<()> {
    rows(block, |name, key, value| {
        db.open_tree(name)?.insert(key, value)?;
        Ok(())
    })
}

fn import_into(
    args: &Source,
    source: &str,
    revision: &str,
    input: &sled::Db,
    db: &sled::Db,
    mut write: impl FnMut(&sled::Db, VerifiedBlock<'_>) -> Result<()>,
) -> Result<Report> {
    let meta = db.open_tree(META)?;
    ensure!(
        !meta.contains_key(COMPLETE_KEY)?,
        "target already contains a completed import"
    );
    for name in DATA {
        db.open_tree(name)?.clear()?;
    }
    let mut report = walk::walk_db(args, input, |block| write(db, block))?;
    db.flush()?;
    // Verify every persisted value against a second walk while retaining the source lock.
    walk::walk_db(args, input, |block| {
        rows(block, |name, key, value| {
            ensure!(
                db.open_tree(name)?.get(key)?.as_deref() == Some(value),
                "readback mismatch in {name}"
            );
            Ok(())
        })
    })?;
    for name in DATA {
        let count = expected_rows(name, args.terminal.terminal_slot, report.counts.blocks);
        ensure!(
            db.open_tree(name)?.len() as u64 == count,
            "unexpected row count in {name}"
        );
    }
    let checksum = digest(db)?;
    let record = CompleteRecord {
        format_version: FORMAT_VERSION,
        source: source.to_owned(),
        importer_revision: revision.to_owned(),
        imported_at_unix_seconds: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        codec_revision: "048c5bf88bd02e19d60d508e0f9f3e1b13a24510".to_owned(),
        genesis_id: args.genesis_id,
        terminal_id: args.terminal.terminal_id,
        terminal_slot: args.terminal.terminal_slot,
        terminal_epoch: args.terminal.terminal_epoch,
        blocks: report.counts.blocks,
        write_batches: report.counts.write_batches,
        content_sha256: checksum,
    };
    meta.insert(COMPLETE_KEY, serde_json::to_vec(&record)?)?;
    db.flush()?;
    report.content_sha256 = Some(record.content_sha256.clone());
    report.complete = Some(record);
    Ok(report)
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        env,
        fs::{self, OpenOptions},
        process::{self, Command},
    };

    use sha2::{Digest, Sha256};
    use tempfile::tempdir;

    use super::*;
    use crate::{Terminal, crash_fixture::Fixture, parse_id};

    // The child receives a private source and target. Exiting without unwinding drops neither
    // the sled handles nor the importer stack, just like a stopped process during the write walk.
    #[test]
    #[ignore = "spawned by the crash-recovery test"]
    fn stop_during_write() {
        let root = Path::new(&env::var_os("STRATA_CRASH_FIXTURE").unwrap()).to_owned();
        let args = source(&root.join("source"));
        let input = walk::open_source(&args).unwrap();
        let db = open_target(&root.join("target"), args.terminal.commitment()).unwrap();
        import_into(&args, "test", REVISION, &input, &db, |db, block| {
            persist_block(db, block)?;
            db.flush()?;
            process::exit(86)
        })
        .unwrap();
    }

    const REVISION: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn source(path: &Path) -> Source {
        Source {
            datadir: path.to_owned(),
            genesis_id: parse_id(
                "7ff1c08429688b0ff921eafb75898c10ba8ade25ffd5317cb3ba0c3a414a7cd1",
            )
            .unwrap(),
            terminal: Terminal {
                terminal_slot: 2,
                terminal_epoch: 1,
                terminal_id: parse_id(
                    "080b6a9d69059feb4e6aa132ed168719c166046c67424a60aa647c993fbbcf1b",
                )
                .unwrap(),
            },
        }
    }

    fn other_trees(db: &sled::Db) -> BTreeMap<Vec<u8>, Vec<u8>> {
        db.tree_names()
            .into_iter()
            .filter(|name| {
                !DATA
                    .into_iter()
                    .chain([META])
                    .any(|archive| archive.as_bytes() == name.as_ref())
            })
            .map(|name| {
                let mut hash = Sha256::new();
                for row in db.open_tree(&name).unwrap().iter() {
                    let (k, v) = row.unwrap();
                    for field in [k, v] {
                        hash.update((field.len() as u64).to_be_bytes());
                        hash.update(field.as_ref());
                    }
                }
                (name.to_vec(), hash.finalize().to_vec())
            })
            .collect()
    }

    #[test]
    fn abrupt_exit_retry_and_removal_preserve_every_other_tree() {
        let root = tempdir().unwrap();
        let fixture = Fixture::new();
        let args = source(&root.path().join("source"));
        assert_eq!(
            (args.genesis_id, args.terminal.terminal_id),
            (fixture.genesis, fixture.terminal)
        );
        fs::rename(fixture.dir.keep(), &args.datadir).unwrap();
        let target = root.path().join("target");
        let path = target.join("sled").join(SLED_NAME);
        let reopen = || {
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .open(path.join("db"))
                .unwrap();
            file.lock().unwrap();
            file.unlock().unwrap();
            sled::open(&path).unwrap()
        };
        let expected = {
            let db = sled::open(&path).unwrap();
            // Logical export from the reader's promote_to_history_anchor plus two live successors.
            let trees: BTreeMap<String, Vec<(String, String)>> =
                serde_json::from_str(include_str!("../testdata/promoted.json")).unwrap();
            for (name, rows) in trees {
                let tree = db.open_tree(name).unwrap();
                for (k, v) in rows {
                    tree.insert(hex::decode(k).unwrap(), hex::decode(v).unwrap())
                        .unwrap();
                }
            }
            // Similar names outside the six-tree contract must survive.
            db.open_tree("V0OLOther")
                .unwrap()
                .insert(b"x", b"keep")
                .unwrap();
            other_trees(&db)
        };
        let status = Command::new(env::current_exe().unwrap())
            .args(["--ignored", "--exact", "import::tests::stop_during_write"])
            .env("STRATA_CRASH_FIXTURE", root.path())
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(86));
        {
            let db = reopen();
            assert_eq!(db.open_tree(BLOCKS).unwrap().len(), 1);
            assert!(
                db.open_tree(META)
                    .unwrap()
                    .get(COMPLETE_KEY)
                    .unwrap()
                    .is_none()
            );
            assert_eq!(other_trees(&db), expected);
            db.open_tree(TXS)
                .unwrap()
                .insert(b"obsolete", b"old row")
                .unwrap();
        }
        let retry = || {
            let report =
                serde_json::to_value(import(&args, &target, "test", REVISION).unwrap()).unwrap();
            assert_eq!(report["txids_sha256_tip_to_genesis"], fixture.txids_digest);
            assert!(
                reopen()
                    .open_tree(BLOCKS)
                    .unwrap()
                    .contains_key(fixture.middle.as_ref())
                    .unwrap()
            );
            report["content_sha256"].as_str().unwrap().to_owned()
        };
        let digest = retry();
        assert_eq!(other_trees(&reopen()), expected);
        let wrong = EpochCommitment::new(0, 0, args.genesis_id);
        assert!(remove(&target, wrong).is_err());
        assert!(
            reopen()
                .open_tree(META)
                .unwrap()
                .get(COMPLETE_KEY)
                .unwrap()
                .is_some()
        );
        for _ in 0..2 {
            remove(&target, args.terminal.commitment()).unwrap();
            let db = reopen();
            assert_eq!(other_trees(&db), expected);
            assert_eq!(db.tree_names().len(), expected.len());
        }
        assert_eq!(retry(), digest);
        assert_eq!(other_trees(&reopen()), expected);
    }
}
