//! Checks that a V0 sled datadir contains the history required by the explorer archive.

use std::{collections::HashSet, path::PathBuf, time::Instant};

use anyhow::{Context, Result, ensure};
use argh::FromArgs;
use serde::Serialize;
use sha2::{Digest, Sha256};
use strata_db_store_sled::{
    SLED_NAME, SledDbConfig, ol::db::OLBlockDBSled, ol_state::db::OLStateDBSled, open_sled_database,
};
use strata_db_types::{ol_block::OLBlockDatabase, ol_state::OLStateDatabase};
use strata_identifiers::{Buf32, Epoch, OLBlockCommitment, OLBlockId, Slot};
// Cargo also passes integration-test dependencies to the binary's test harness.
#[cfg(test)]
use {sled as _, strata_ol_chain_types as _, strata_ol_state_types as _, tempfile as _};

/// Verify a disposable, offline copy of an existing v0.3.5 node datadir.
/// Sled requires exclusive, writable access for locking and recovery.
#[derive(FromArgs)]
struct Args {
    /// root of the copied datadir containing sled/strata-client
    #[argh(option)]
    datadir: PathBuf,
    /// expected 32-byte genesis block ID in hex
    #[argh(option, from_str_fn(parse_id))]
    genesis_id: OLBlockId,
    /// expected 32-byte promotion terminal block ID in hex
    #[argh(option, from_str_fn(parse_id))]
    terminal_id: OLBlockId,
    /// slot of the promotion terminal
    #[argh(option)]
    terminal_slot: Slot,
    /// epoch of the promotion terminal
    #[argh(option)]
    terminal_epoch: Epoch,
}

fn parse_id(input: &str) -> Result<OLBlockId, String> {
    input
        .parse::<Buf32>()
        .map(Into::into)
        .map_err(|e| e.to_string())
}

#[derive(Serialize, Default)]
struct Counts {
    blocks: u64,
    write_batches: u64,
    transactions: u64,
    created_accounts: u64,
    updated_accounts: u64,
}

#[derive(Serialize)]
struct Report {
    genesis_id: String,
    terminal_id: String,
    terminal_slot: Slot,
    terminal_epoch: Epoch,
    counts: Counts,
    /// SHA-256 over concatenated V0 transaction IDs, newest block first, in body order.
    txids_sha256_tip_to_genesis: String,
    elapsed_ms: u128,
}

fn verify(args: Args) -> Result<Report> {
    let started = Instant::now();
    let db_path = args.datadir.join("sled").join(SLED_NAME).join("db");
    ensure!(
        db_path.is_file(),
        "missing sled data file {}",
        db_path.display()
    );
    let db = open_sled_database(&args.datadir, SLED_NAME)?;
    let config = SledDbConfig::production();
    let blocks = OLBlockDBSled::new(db.clone(), config.clone())?;
    let states = OLStateDBSled::new(db, config)?;
    let mut id = args.terminal_id;
    let mut counts = Counts::default();
    let mut txids = Sha256::new();

    for slot in (0..=args.terminal_slot).rev() {
        let at = || format!("slot {slot} block {}", hex::encode(id.as_ref()));
        let block = blocks
            .get_block_data(id)
            .with_context(|| format!("{}: reading V0 block", at()))?
            .with_context(|| format!("{}: missing block", at()))?;
        let header = block.header();
        ensure!(
            header.slot() == slot,
            "{}: header has slot {}",
            at(),
            header.slot()
        );
        ensure!(
            header.compute_blkid() == id,
            "{}: header block ID mismatch",
            at()
        );
        ensure!(
            block.body().compute_hash_commitment() == *header.body_root(),
            "{}: body root mismatch",
            at()
        );
        if slot == args.terminal_slot {
            ensure!(header.is_terminal(), "{}: terminal flag absent", at());
            ensure!(
                header.epoch() == args.terminal_epoch,
                "{}: expected epoch {}, found {}",
                at(),
                args.terminal_epoch,
                header.epoch()
            );
        }
        if let Some(segment) = block.body().tx_segment() {
            for tx in segment.txs() {
                txids.update(tx.compute_txid().as_ref());
                counts.transactions += 1;
            }
        }
        counts.blocks += 1;

        let batch = states
            .get_ol_write_batch(OLBlockCommitment::new(slot, id))
            .with_context(|| format!("{}: reading V0 write batch", at()))?;
        if slot == 0 {
            ensure!(
                id == args.genesis_id,
                "{}: expected genesis {}",
                at(),
                hex::encode(args.genesis_id.as_ref())
            );
            ensure!(batch.is_none(), "{}: unexpected genesis write batch", at());
            break;
        }
        let batch = batch.with_context(|| format!("{}: missing write batch", at()))?;
        let ledger = batch.ledger();
        let created: HashSet<_> = ledger.new_accounts().iter().copied().collect();
        for (account, _) in ledger.iter_accounts() {
            if created.contains(account) {
                counts.created_accounts += 1;
            } else {
                counts.updated_accounts += 1;
            }
        }
        counts.write_batches += 1;
        id = *header.parent_blkid();
    }
    Ok(Report {
        genesis_id: hex::encode(args.genesis_id.as_ref()),
        terminal_id: hex::encode(args.terminal_id.as_ref()),
        terminal_slot: args.terminal_slot,
        terminal_epoch: args.terminal_epoch,
        counts,
        txids_sha256_tip_to_genesis: hex::encode(txids.finalize()),
        elapsed_ms: started.elapsed().as_millis(),
    })
}

fn main() -> Result<()> {
    let report = verify(argh::from_env())?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
