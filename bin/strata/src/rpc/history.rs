//! Read-only explorer access to the V0 trees written by the offline v0.3.5 importer.
mod layout;

use anyhow::{Context, Result, anyhow, ensure};
use layout::*;
use serde::Deserialize;
use ssz::Decode;
use strata_identifiers::{Epoch, EpochCommitment, OLBlockId, Slot};
use strata_ol_chain_types_v1::OLBlockHeaderV1;
use strata_ol_rpc_types::{
    RpcBlockAccountChanges, RpcBlockEntry, RpcBlockHeaderEntry, RpcOLTxDetail,
};
use strata_primitives::HexBytes;
use tokio::task;

#[derive(Deserialize)]
struct Manifest {
    format_version: u32,
    genesis_id: OLBlockId,
    terminal_slot: Slot,
    terminal_epoch: Epoch,
    terminal_id: OLBlockId,
    blocks: u64,
    write_batches: u64,
    content_sha256: String,
}

/// The verified, immutable V0 prefix of the promoted node.
#[derive(Debug, Clone)]
pub(crate) struct HistoryArchive {
    headers: sled::Tree,
    blocks: sled::Tree,
    transactions: sled::Tree,
    changes: sled::Tree,
    genesis: OLBlockId,
    terminal: EpochCommitment,
}

impl HistoryArchive {
    /// Opens a completed import and verifies its contents against the node's own anchors.
    pub(crate) fn discover(
        db: &sled::Db,
        anchors: impl FnOnce() -> Result<(OLBlockId, EpochCommitment)>,
    ) -> Result<Option<Self>> {
        let names = db.tree_names();
        let present = |name: &str| names.iter().any(|n| n.as_ref() == name.as_bytes());
        if !present(META) {
            return Ok(None);
        }
        let Some(record) = db.open_tree(META)?.get(b"complete")? else {
            return Ok(None);
        };
        let manifest: Manifest =
            serde_json::from_slice(&record).context("reading V0 archive completion record")?;
        ensure!(
            manifest.format_version == 1,
            "unsupported V0 archive format"
        );
        let (genesis, terminal) = anchors()?;
        ensure!(
            manifest.genesis_id == genesis,
            "V0 archive genesis mismatch"
        );
        ensure!(
            EpochCommitment::new(
                manifest.terminal_epoch,
                manifest.terminal_slot,
                manifest.terminal_id
            ) == terminal,
            "V0 archive terminal differs from the node history base"
        );
        let last = terminal.last_slot();
        ensure!(
            last.checked_add(1) == Some(manifest.blocks) && manifest.write_batches == last,
            "V0 archive coverage mismatch"
        );
        for name in DATA {
            ensure!(present(name), "missing V0 archive tree {name}");
            let expected = if [CHANGES, BATCHES].contains(&name) {
                last
            } else {
                manifest.blocks
            };
            ensure!(
                db.open_tree(name)?.len() as u64 == expected,
                "wrong row count in {name}"
            );
        }
        ensure!(
            digest(db)? == manifest.content_sha256,
            "V0 archive checksum mismatch"
        );
        let batches = db.open_tree(BATCHES)?;
        let archive = Self {
            headers: db.open_tree(HEADERS)?,
            blocks: db.open_tree(BLOCKS)?,
            transactions: db.open_tree(TXS)?,
            changes: db.open_tree(CHANGES)?,
            genesis,
            terminal,
        };
        let mut expected_id = *terminal.last_blkid();
        for slot in (0..=last).rev() {
            let header = archive.header(slot)?;
            ensure!(
                header.compute_blkid() == expected_id,
                "V0 header hash mismatch at slot {slot}"
            );
            if slot == last {
                ensure!(
                    header.epoch() == terminal.epoch() && header.is_terminal(),
                    "invalid V0 terminal header"
                );
            }
            if slot == 0 {
                ensure!(
                    expected_id == genesis && header.epoch() == 0 && header.is_terminal(),
                    "invalid V0 genesis header"
                );
            }
            let id = expected_id.as_ref();
            ensure!(
                archive.blocks.contains_key(id)? && archive.transactions.contains_key(id)?,
                "missing V0 block or transactions at slot {slot}"
            );
            ensure!(
                batches.contains_key(id)? == (slot != 0)
                    && archive.changes.contains_key(id)? == (slot != 0),
                "incorrect V0 batch coverage at slot {slot}"
            );
            expected_id = *header.parent_blkid();
        }
        Ok(Some(archive))
    }

    pub(super) fn genesis(&self) -> OLBlockId {
        self.genesis
    }
    pub(super) fn terminal(&self) -> EpochCommitment {
        self.terminal
    }

    fn header(&self, slot: Slot) -> Result<OLBlockHeaderV1> {
        let raw = self
            .headers
            .get(slot.to_be_bytes())?
            .context("missing V0 header")?;
        let header = OLBlockHeaderV1::from_ssz_bytes(&raw)
            .map_err(|e| anyhow!("invalid V0 header at slot {slot}: {e:?}"))?;
        ensure!(header.slot() == slot, "wrong V0 header slot {slot}");
        Ok(header)
    }

    async fn read<T: Send + 'static>(
        &self,
        query: impl FnOnce(&Self) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let archive = self.clone();
        task::spawn_blocking(move || query(&archive)).await?
    }

    pub(super) async fn blocks(&self, start: Slot, end: Slot) -> Result<Vec<RpcBlockEntry>> {
        self.read(move |db| {
            (start..=end)
                .map(|slot| {
                    let header = db.header(slot)?;
                    let id = header.compute_blkid();
                    let raw = db.blocks.get(id.as_ref())?.context("missing V0 block")?;
                    Ok(RpcBlockEntry::new(
                        slot,
                        header.epoch(),
                        id,
                        HexBytes(raw.to_vec()),
                    ))
                })
                .collect()
        })
        .await
    }

    pub(super) async fn headers(&self, start: Slot, end: Slot) -> Result<Vec<RpcBlockHeaderEntry>> {
        self.read(move |db| {
            (start..=end)
                .map(|slot| Ok((&db.header(slot)?).into()))
                .collect()
        })
        .await
    }

    pub(super) async fn raw_block(&self, id: OLBlockId) -> Result<Option<HexBytes>> {
        self.read(move |db| {
            Ok(db
                .blocks
                .get(id.as_ref())?
                .map(|raw| HexBytes(raw.to_vec())))
        })
        .await
    }

    pub(super) async fn transactions(&self, slot: Slot) -> Result<Vec<RpcOLTxDetail>> {
        self.read(move |db| {
            let row = db
                .transactions
                .get(db.header(slot)?.compute_blkid().as_ref())?
                .context("missing V0 transactions")?;
            Ok(serde_json::from_slice(&row)?)
        })
        .await
    }

    pub(super) async fn account_changes(&self, slot: Slot) -> Result<RpcBlockAccountChanges> {
        self.read(move |db| {
            let row = db
                .changes
                .get(db.header(slot)?.compute_blkid().as_ref())?
                .context("missing V0 account changes")?;
            Ok(serde_json::from_slice(&row)?)
        })
        .await
    }
}
