//! Imports only canonical V0 history into an offline, promoted node.

use std::path::Path;

use anyhow::{Context, Result, ensure};
use ssz::Encode;
use strata_codec::decode_buf_exact;
use strata_db_store_sled::{
    SLED_NAME,
    ol::schemas::v0_history::{
        BATCHES, BLOCKS, CHANGES, COMPLETE_KEY, CompleteRecord, DATA, FORMAT_VERSION, HEADERS,
        META, TXS, digest,
    },
};
use strata_identifiers::EpochCommitment;

use crate::{
    Args,
    walk::{self, Report, VerifiedBlock},
};

fn rows(
    block: VerifiedBlock<'_>,
    mut row: impl FnMut(&str, &[u8], &[u8]) -> Result<()>,
) -> Result<()> {
    let header = block.block.header();
    let id = header.compute_blkid();
    let key = id.as_ref();
    row(
        HEADERS,
        &header.slot().to_be_bytes(),
        &header.as_ssz_bytes(),
    )?;
    row(BLOCKS, key, block.raw)?;
    row(TXS, key, &serde_json::to_vec(&block.transactions)?)?;
    if let Some((batch, changes)) = block.batch {
        row(BATCHES, key, batch)?;
        row(CHANGES, key, &serde_json::to_vec(&changes)?)?;
    }
    Ok(())
}

pub(super) fn import(args: &Args, target: &Path, source: &str) -> Result<Report> {
    let path = target.join("sled").join(SLED_NAME);
    ensure!(path.join("db").is_file(), "missing target sled database");
    ensure!(
        path.canonicalize()? != args.datadir.join("sled").join(SLED_NAME).canonicalize()?,
        "source and target must be different databases"
    );
    let input = walk::open_source(args)?;
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
        base == EpochCommitment::new(args.terminal_epoch, args.terminal_slot, args.terminal_id),
        "target history base differs from the requested terminal"
    );
    let meta = db.open_tree(META)?;
    ensure!(
        !meta.contains_key(COMPLETE_KEY)?,
        "target already contains a completed import"
    );
    for name in DATA {
        db.open_tree(name)?;
    }
    let mut report = walk::walk_db(args, &input, |block| {
        rows(block, |name, key, value| {
            db.open_tree(name)?.insert(key, value)?;
            Ok(())
        })
    })?;
    db.flush()?;
    // Verify every persisted value against a second walk while retaining the source lock.
    walk::walk_db(args, &input, |block| {
        rows(block, |name, key, value| {
            ensure!(
                db.open_tree(name)?.get(key)?.as_deref() == Some(value),
                "readback mismatch in {name}"
            );
            Ok(())
        })
    })?;
    for name in DATA {
        let count = if [BATCHES, CHANGES].contains(&name) {
            report.counts.write_batches
        } else {
            report.counts.blocks
        };
        ensure!(
            db.open_tree(name)?.len() as u64 == count,
            "unexpected row count in {name}"
        );
    }
    let checksum = digest(&db)?;
    let record = CompleteRecord {
        format_version: FORMAT_VERSION,
        source: source.to_owned(),
        decoder_revision: "048c5bf88bd02e19d60d508e0f9f3e1b13a24510".to_owned(),
        genesis_id: args.genesis_id,
        terminal_id: args.terminal_id,
        terminal_slot: args.terminal_slot,
        terminal_epoch: args.terminal_epoch,
        blocks: report.counts.blocks,
        write_batches: report.counts.write_batches,
        content_sha256: checksum,
    };
    meta.insert(COMPLETE_KEY, serde_json::to_vec(&record)?)?;
    db.flush()?;
    report.content_sha256 = Some(record.content_sha256);
    Ok(report)
}
