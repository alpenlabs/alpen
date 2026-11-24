use std::{sync::Arc, time::Instant};

use anyhow::{Context, Result, anyhow, ensure};
use serde::Serialize;
use sha2::{Digest, Sha256};
use ssz::Decode;
use strata_db_store_sled::{SLED_NAME, SledDbConfig, ol_state::db::OLStateDBSled};
use strata_db_types::{DbError, ol_state::OLStateDatabase};
use strata_identifiers::{Epoch, OLBlockCommitment, Slot};
use strata_ol_chain_types::OLBlock;
use strata_ol_rpc_types::{RpcAccountChangeType, RpcBlockAccountChanges, RpcOLTxDetail};
use typed_sled::SledDb;

use super::Args;

#[derive(Serialize, Default)]
pub(super) struct Counts {
    blocks: u64,
    write_batches: u64,
    transactions: u64,
    created_accounts: u64,
    updated_accounts: u64,
}

#[derive(Serialize)]
pub(super) struct Report {
    genesis_id: String,
    terminal_id: String,
    terminal_slot: Slot,
    terminal_epoch: Epoch,
    pub(super) counts: Counts,
    /// SHA-256 over concatenated V0 transaction IDs, newest block first, in body order.
    txids_sha256_tip_to_genesis: String,
    elapsed_ms: u128,
}

pub(super) struct VerifiedBlock<'a> {
    pub block: &'a OLBlock,
    pub raw: &'a [u8],
    pub transactions: Vec<RpcOLTxDetail>,
    pub account_changes: Option<RpcBlockAccountChanges>,
}

pub(super) fn walk(
    args: &Args,
    mut visit: impl FnMut(VerifiedBlock<'_>) -> Result<()>,
) -> Result<Report> {
    let started = Instant::now();
    let db_path = args.datadir.join("sled").join(SLED_NAME).join("db");
    ensure!(
        db_path.is_file(),
        "missing sled data file {}",
        db_path.display()
    );
    // The typed block getter decodes SSZ and discards the source buffer. Keep that
    // buffer for raw range and by-ID responses, including the original signature.
    let db = sled::open(args.datadir.join("sled").join(SLED_NAME))
        .context("opening V0 sled database")?;
    let blocks = db.open_tree("OLBlockSchema")?;
    let states = OLStateDBSled::new(
        Arc::new(SledDb::new(db).map_err(DbError::from)?),
        SledDbConfig::production(),
    )?;
    let mut id = args.terminal_id;
    let mut counts = Counts::default();
    let mut txids = Sha256::new();

    for slot in (0..=args.terminal_slot).rev() {
        let at = || format!("slot {slot} block {}", hex::encode(id.as_ref()));
        let raw = blocks
            .get(id.as_ref())
            .with_context(|| format!("{}: reading V0 block", at()))?
            .with_context(|| format!("{}: missing block", at()))?;
        let block = OLBlock::from_ssz_bytes(&raw)
            .map_err(|e| anyhow!("{}: reading V0 block: {e:?}", at()))?;
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
        let transactions: Vec<_> = block
            .body()
            .tx_segment()
            .into_iter()
            .flat_map(|segment| segment.txs())
            .map(RpcOLTxDetail::from)
            .collect();
        for tx in &transactions {
            txids.update(tx.txid().as_ref());
        }
        counts.transactions += transactions.len() as u64;
        counts.blocks += 1;

        let commitment = OLBlockCommitment::new(slot, id);
        let batch = states
            .get_ol_write_batch(commitment)
            .with_context(|| format!("{}: reading V0 write batch", at()))?;
        let account_changes = if slot == 0 {
            ensure!(
                id == args.genesis_id,
                "{}: expected genesis {}",
                at(),
                hex::encode(args.genesis_id.as_ref())
            );
            ensure!(batch.is_none(), "{}: unexpected genesis write batch", at());
            None
        } else {
            let batch = batch.with_context(|| format!("{}: missing write batch", at()))?;
            let changes = RpcBlockAccountChanges::from_write_batch(commitment, &batch);
            for change in changes.changes() {
                match change.change_type() {
                    RpcAccountChangeType::Created => counts.created_accounts += 1,
                    RpcAccountChangeType::Updated => counts.updated_accounts += 1,
                }
            }
            counts.write_batches += 1;
            Some(changes)
        };
        visit(VerifiedBlock {
            block: &block,
            raw: &raw,
            transactions,
            account_changes,
        })
        .with_context(|| format!("{}: processing verified block", at()))?;
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
