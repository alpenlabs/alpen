//! Read-only explorer access to the V0 trees written by the offline v0.3.5 importer.

use std::time::Instant;

use anyhow::{Context, Result, anyhow, ensure};
use sha2::{Digest, Sha256};
use ssz::{Decode, DecodeError, SszDecoderBuilder};
use strata_db_store_sled::ol::schemas::v0_history::{
    BATCHES, BLOCKS, CHANGES, COMPLETE_KEY, CompleteRecord, DATA, FORMAT_VERSION, HEADERS, META,
    TXS, digest, expected_rows, header_key,
};
use strata_identifiers::{EpochCommitment, OLBlockId, Slot};
use strata_ol_chain_types_v1::{OLBlockHeaderV1, SignedOLBlockHeaderV1};
use strata_ol_rpc_types::{
    RpcBlockAccountChanges, RpcBlockEntry, RpcBlockHeaderEntry, RpcOLTxDetail,
};
use strata_primitives::HexBytes;
use tokio::task;
use tracing::info;

/// Only the signed-header layout is common to V0 and V1. Hash the original V0 body bytes.
fn read_envelope(bytes: &[u8]) -> Result<(SignedOLBlockHeaderV1, [u8; 32]), DecodeError> {
    let mut layout = SszDecoderBuilder::new(bytes);
    layout.register_type::<SignedOLBlockHeaderV1>()?;
    layout.register_anonymous_variable_length_item()?;
    let mut fields = layout.build()?;
    Ok((
        fields.decode_next()?,
        fields.decode_next_with(|body| Ok(Sha256::digest(body).into()))?,
    ))
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
        let start = Instant::now();
        let names = db.tree_names();
        let present = |name: &str| names.iter().any(|n| n.as_ref() == name.as_bytes());
        if !present(META) {
            return Ok(None);
        }
        let Some(record) = db.open_tree(META)?.get(COMPLETE_KEY)? else {
            info!("V0 import has no completion record");
            return Ok(None);
        };
        let manifest: CompleteRecord =
            serde_json::from_slice(&record).context("reading V0 archive completion record")?;
        ensure!(
            manifest.format_version == FORMAT_VERSION,
            "unsupported V0 archive format: expected {FORMAT_VERSION}, found {}",
            manifest.format_version
        );
        let (genesis, terminal) = anchors()?;
        ensure!(
            manifest.genesis_id == genesis,
            "V0 archive genesis mismatch: expected {genesis:?}, found {:?}",
            manifest.genesis_id
        );
        ensure!(
            EpochCommitment::new(
                manifest.terminal_epoch,
                manifest.terminal_slot,
                manifest.terminal_id
            ) == terminal,
            "V0 archive terminal differs: expected {terminal:?}, found epoch {} slot {} ID {:?}",
            manifest.terminal_epoch,
            manifest.terminal_slot,
            manifest.terminal_id
        );
        let last = terminal.last_slot();
        ensure!(
            last.checked_add(1) == Some(manifest.blocks) && manifest.write_batches == last,
            "V0 archive coverage mismatch: terminal slot {last}, found {} blocks and {} batches",
            manifest.blocks,
            manifest.write_batches
        );
        for name in DATA {
            ensure!(present(name), "missing V0 archive tree {name}");
            let expected = expected_rows(name, last, manifest.blocks);
            let actual = db.open_tree(name)?.len() as u64;
            ensure!(
                actual == expected,
                "wrong row count in {name}: expected {expected}, found {actual}"
            );
        }
        let actual = digest(db)?;
        ensure!(
            actual == manifest.content_sha256,
            "V0 archive checksum mismatch: expected {}, found {actual}",
            manifest.content_sha256
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
            let raw = archive
                .blocks
                .get(id)?
                .with_context(|| format!("missing block at slot {slot}"))?;
            let (signed, body_hash) = read_envelope(&raw)
                .map_err(|e| anyhow!("invalid V0 block at slot {slot}: {e:?}"))?;
            ensure!(
                signed.header() == &header,
                "signed block header differs at slot {slot}"
            );
            ensure!(
                body_hash.as_slice() == header.body_root().as_ref(),
                "V0 body hash mismatch at slot {slot}"
            );
            let txs = archive
                .transactions
                .get(id)?
                .with_context(|| format!("missing transactions at slot {slot}"))?;
            serde_json::from_slice::<Vec<RpcOLTxDetail>>(&txs)
                .with_context(|| format!("invalid transactions at slot {slot}"))?;
            let changes = archive.changes.get(id)?;
            ensure!(
                batches.contains_key(id)? == (slot != 0) && changes.is_some() == (slot != 0),
                "incorrect V0 batch coverage at slot {slot}"
            );
            if let Some(bytes) = changes {
                let row: RpcBlockAccountChanges = serde_json::from_slice(&bytes)
                    .with_context(|| format!("invalid account changes at slot {slot}"))?;
                ensure!(
                    row.slot() == slot && row.blkid() == expected_id,
                    "account changes reference a different block at slot {slot}"
                );
            }
            expected_id = *header.parent_blkid();
        }
        info!(?terminal, blocks = manifest.blocks, checksum = %actual, elapsed_ms = start.elapsed().as_millis(), "verified V0 explorer archive");
        Ok(Some(archive))
    }

    pub(super) fn genesis(&self) -> OLBlockId {
        self.genesis
    }
    pub(super) fn terminal(&self) -> EpochCommitment {
        self.terminal
    }

    pub(super) fn last_slot(&self) -> Slot {
        self.terminal.last_slot()
    }

    fn header(&self, slot: Slot) -> Result<OLBlockHeaderV1> {
        let raw = self
            .headers
            .get(header_key(slot))?
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
