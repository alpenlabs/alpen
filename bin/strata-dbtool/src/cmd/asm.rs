//! Offline removal of ASM replay data without decoding legacy records.

use std::path::Path;

use argh::FromArgs;
use serde::Serialize;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db_store_sled::{asm::ASM_REPLAY_TABLE_NAMES, SLED_NAME};

use crate::{
    cli::OutputFormat,
    output::{output, traits::Formattable},
};

/// Reset ASM state, logs and auxiliary data for L1 replay.
///
/// Stop the node and back up its database first. Other tables are retained.
#[derive(FromArgs, Debug)]
#[argh(subcommand, name = "reset-asm")]
pub(crate) struct ResetAsmArgs {
    /// delete the three ASM tables (without this flag, only preview row counts)
    #[argh(switch, short = 'f')]
    pub(crate) force: bool,

    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

#[derive(Debug, Serialize)]
struct AsmTableReset {
    name: &'static str,
    present: bool,
    entries: usize,
}

#[derive(Serialize)]
struct AsmResetReport<'a> {
    database: &'a Path,
    dry_run: bool,
    tables: Vec<AsmTableReset>,
}

impl Formattable for AsmResetReport<'_> {
    fn format_porcelain(&self) -> String {
        let mut lines = vec![
            format!("database: {}", self.database.display()),
            format!("dry_run: {}", self.dry_run),
        ];
        for table in &self.tables {
            lines.push(format!(
                "{}: present={} entries={}",
                table.name, table.present, table.entries
            ));
        }
        if self.dry_run {
            lines.push(
                "Use --force to delete these ASM tables after backing up the stopped node.".into(),
            );
        } else {
            lines.push(
                "ASM reset flushed. Keep sequencing and proving disabled during replay.".into(),
            );
        }
        lines.join("\n")
    }
}

/// Opens an existing OL database and previews or deletes only the ASM replay tables.
pub(crate) fn reset_asm(datadir: &Path, args: ResetAsmArgs) -> Result<(), DisplayedError> {
    let database = datadir.join("sled").join(SLED_NAME);
    // Sled's normal open creates a database. A reset must reject typos rather than
    // reporting success against a newly created, empty store.
    if !database.join("conf").is_file() || !database.join("db").is_file() {
        return Err(DisplayedError::UserError(
            "Expected an existing OL sled database".into(),
            Box::new(database),
        ));
    }
    // Avoid SledBackend initialization: it creates unrelated trees, and this
    // operation must also work with records that the current codecs cannot read.
    let db = sled::open(&database)
        .user_error("Could not open OL database; stop the node before resetting ASM")?;
    let tables = reset_tables(&db, args.force)?;
    output(
        &AsmResetReport {
            database: &database,
            dry_run: !args.force,
            tables,
        },
        args.output_format,
    )
}

fn reset_tables(db: &sled::Db, force: bool) -> Result<Vec<AsmTableReset>, DisplayedError> {
    let existing = db.tree_names();
    let mut tables = Vec::with_capacity(ASM_REPLAY_TABLE_NAMES.len());
    for name in ASM_REPLAY_TABLE_NAMES {
        let present = existing.iter().any(|tree| tree.as_ref() == name.as_bytes());
        let entries = if present {
            let tree = db
                .open_tree(name)
                .internal_error("Could not inspect ASM table")?;
            tree.iter()
                .keys()
                .try_fold(0usize, |count, key| key.map(|_| count + 1))
                .internal_error("Could not count ASM rows; no tables were deleted")?
        } else {
            0
        };
        tables.push(AsmTableReset {
            name,
            present,
            entries,
        });
    }

    if force {
        // Drops are not a transaction across trees. Repeating the command removes
        // any remaining tables after interruption, without decoding or recreating them.
        for table in &tables {
            if table.present {
                db.drop_tree(table.name).internal_error(
                    "ASM reset incomplete; rerun reset-asm --force before restarting the node",
                )?;
            }
        }
        db.flush().internal_error(
            "Could not flush ASM reset; rerun reset-asm --force before restarting the node",
        )?;
    }
    Ok(tables)
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, fs};

    use tempfile::{tempdir, TempDir};

    use super::*;
    use crate::cli::{Cli, Command};

    fn seeded_database() -> (TempDir, sled::Db) {
        let dir = tempdir().unwrap();
        let db = sled::open(dir.path().join("sled").join(SLED_NAME)).unwrap();
        for name in ASM_REPLAY_TABLE_NAMES {
            // Intentionally opaque legacy/corrupt values: reset must not decode them.
            db.open_tree(name)
                .unwrap()
                .insert(b"old-key", &[255, 0, 123])
                .unwrap();
        }
        for name in [
            "L1BlockSchema",
            "ClientStateSchema",
            "ManifestMmr",
            "CheckpointProofSchema",
            "BroadcastQueue",
            "unrelated",
        ] {
            db.open_tree(name)
                .unwrap()
                .insert(b"retained-key", b"retained-value")
                .unwrap();
        }
        db.insert(b"metadata", b"retained").unwrap();
        db.flush().unwrap();
        (dir, db)
    }

    type Snapshot = BTreeMap<Vec<u8>, Vec<(Vec<u8>, Vec<u8>)>>;

    fn snapshot(db: &sled::Db) -> Snapshot {
        db.tree_names()
            .into_iter()
            .map(|name| {
                let rows = db
                    .open_tree(&name)
                    .unwrap()
                    .iter()
                    .map(|row| {
                        let (key, value) = row.unwrap();
                        (key.to_vec(), value.to_vec())
                    })
                    .collect();
                (name.to_vec(), rows)
            })
            .collect()
    }

    fn args(force: bool) -> ResetAsmArgs {
        ResetAsmArgs {
            force,
            output_format: OutputFormat::Json,
        }
    }

    #[test]
    fn preview_preserves_all_rows_and_does_not_recreate_missing_tables() {
        let (dir, db) = seeded_database();
        db.drop_tree(ASM_REPLAY_TABLE_NAMES[1]).unwrap();
        let before = snapshot(&db);
        let report = reset_tables(&db, false).unwrap();
        assert_eq!(
            report.iter().map(|table| table.entries).collect::<Vec<_>>(),
            vec![1, 0, 1]
        );
        assert!(!report[1].present);
        drop(db);
        reset_asm(dir.path(), args(false)).unwrap();
        let db = sled::open(dir.path().join("sled").join(SLED_NAME)).unwrap();
        assert_eq!(snapshot(&db), before);
    }

    #[test]
    fn reset_removes_only_asm_tables_and_persists_across_reopen() {
        let (dir, db) = seeded_database();
        let mut expected = snapshot(&db);
        for name in ASM_REPLAY_TABLE_NAMES {
            expected.remove(name.as_bytes());
        }
        drop(db);
        reset_asm(dir.path(), args(true)).unwrap();
        let db = sled::open(dir.path().join("sled").join(SLED_NAME)).unwrap();
        assert_eq!(snapshot(&db), expected);
        let report = reset_tables(&db, true).unwrap();
        assert!(report
            .iter()
            .all(|table| !table.present && table.entries == 0));
        assert_eq!(snapshot(&db), expected);
    }

    #[test]
    fn reset_can_resume_after_one_table_was_removed() {
        let (_dir, db) = seeded_database();
        db.drop_tree(ASM_REPLAY_TABLE_NAMES[0]).unwrap();
        reset_tables(&db, true).unwrap();
        for name in ASM_REPLAY_TABLE_NAMES {
            assert!(!db
                .tree_names()
                .iter()
                .any(|tree| tree.as_ref() == name.as_bytes()));
        }
        assert_eq!(
            db.open_tree("unrelated")
                .unwrap()
                .get(b"retained-key")
                .unwrap()
                .unwrap()
                .as_ref(),
            b"retained-value"
        );
    }

    #[test]
    fn missing_or_empty_database_path_is_rejected_without_creating_database() {
        let dir = tempdir().unwrap();
        assert!(reset_asm(dir.path(), args(true)).is_err());
        assert!(!dir.path().join("sled").exists());
        let database = dir.path().join("sled").join(SLED_NAME);
        fs::create_dir_all(&database).unwrap();
        assert!(reset_asm(dir.path(), args(false)).is_err());
        assert_eq!(fs::read_dir(database).unwrap().count(), 0);
    }

    #[test]
    fn locked_database_is_rejected_without_deleting_rows() {
        let (dir, db) = seeded_database();
        let before = snapshot(&db);
        assert!(reset_asm(dir.path(), args(true)).is_err());
        assert_eq!(snapshot(&db), before);
    }

    #[test]
    fn cli_defaults_to_preview_and_accepts_explicit_force() {
        for (arguments, expected_force) in [
            (vec!["reset-asm"], false),
            (vec!["reset-asm", "--force", "-o", "json"], true),
        ] {
            let cli = Cli::from_args(&["strata-dbtool"], &arguments).unwrap();
            let Command::ResetAsm(args) = cli.cmd else {
                panic!("expected reset-asm command");
            };
            assert_eq!(args.force, expected_force);
        }
    }
}
