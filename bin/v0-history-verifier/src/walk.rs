use std::time::Instant;

use anyhow::{Context, Result, anyhow, ensure};
use serde::Serialize;
use sha2::{Digest, Sha256};
use ssz::Decode;
use strata_codec::{decode_buf_exact, encode_to_vec};
use strata_db_store_sled::SLED_NAME;
use strata_identifiers::{Epoch, OLBlockCommitment, Slot};
use strata_ol_chain_types::OLBlock;
use strata_ol_rpc_types::{RpcAccountChangeType, RpcBlockAccountChanges, RpcOLTxDetail};
use strata_ol_state_types::{OLAccountState, WriteBatch};

use super::Args;

#[derive(Serialize, Default)]
pub(super) struct Counts {
    pub(super) blocks: u64,
    pub(super) write_batches: u64,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) content_sha256: Option<String>,
}

pub(super) struct VerifiedBlock<'a> {
    pub batch: Option<(&'a [u8], RpcBlockAccountChanges)>,
    pub block: &'a OLBlock,
    pub raw: &'a [u8],
    pub transactions: Vec<RpcOLTxDetail>,
}

pub(super) fn walk(
    args: &Args,
    visit: impl FnMut(VerifiedBlock<'_>) -> Result<()>,
) -> Result<Report> {
    let db = open_source(args)?;
    walk_db(args, &db, visit)
}

pub(super) fn open_source(args: &Args) -> Result<sled::Db> {
    let path = args.datadir.join("sled").join(SLED_NAME);
    let db_path = path.join("db");
    ensure!(
        db_path.is_file(),
        "missing sled data file {}",
        db_path.display()
    );
    sled::open(path).context("opening V0 sled database")
}

pub(super) fn walk_db(
    args: &Args,
    db: &sled::Db,
    mut visit: impl FnMut(VerifiedBlock<'_>) -> Result<()>,
) -> Result<Report> {
    let started = Instant::now();
    let blocks = db.open_tree("OLBlockSchema")?;
    let batches = db.open_tree("OLWriteBatchSchema")?;
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
        let raw_batch = batches.get(encode_to_vec(&commitment)?)?;
        let batch = if slot == 0 {
            ensure!(
                id == args.genesis_id,
                "{}: expected genesis {}",
                at(),
                hex::encode(args.genesis_id.as_ref())
            );
            ensure!(
                raw_batch.is_none(),
                "{}: unexpected genesis write batch",
                at()
            );
            None
        } else {
            let bytes = raw_batch
                .as_deref()
                .with_context(|| format!("{}: missing write batch", at()))?;
            let batch: WriteBatch<OLAccountState> = decode_buf_exact(bytes)
                .with_context(|| format!("{}: reading V0 write batch", at()))?;
            let changes = RpcBlockAccountChanges::from_write_batch(commitment, &batch);
            for change in changes.changes() {
                match change.change_type() {
                    RpcAccountChangeType::Created => counts.created_accounts += 1,
                    RpcAccountChangeType::Updated => counts.updated_accounts += 1,
                }
            }
            counts.write_batches += 1;
            Some((bytes, changes))
        };
        visit(VerifiedBlock {
            block: &block,
            raw: &raw,
            batch,
            transactions,
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
        content_sha256: None,
        elapsed_ms: started.elapsed().as_millis(),
    })
}
