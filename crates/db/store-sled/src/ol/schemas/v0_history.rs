//! Format 1 of the V0 explorer archive, shared by the current node and the v0.3.5 importer.
//! The header and transaction trees cover every canonical slot from genesis through the
//! promotion terminal. The batch and account-change trees omit genesis only.
//! An importer flushes all five data trees and verifies them against the source before
//! publishing [`CompleteRecord`] under [`COMPLETE_KEY`]. Readers ignore unpublished trees.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use strata_identifiers::{Epoch, OLBlockId, Slot};
/// Supported on-disk format.
pub const FORMAT_VERSION: u32 = 1;
/// ASCII key of the single completion record.
pub const COMPLETE_KEY: [u8; 8] = *b"complete";

/// UTF-8 JSON value published after verification. Block IDs use their hex JSON representation.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CompleteRecord {
    pub format_version: u32,
    pub source: String,
    #[serde(alias = "decoder_revision")]
    pub codec_revision: String,
    /// Older format-1 records predate importer provenance.
    #[serde(default)]
    pub importer_revision: Option<String>,
    #[serde(default)]
    pub imported_at_unix_seconds: Option<u64>,
    pub genesis_id: OLBlockId,
    pub terminal_slot: Slot,
    pub terminal_epoch: Epoch,
    pub terminal_id: OLBlockId,
    pub blocks: u64,
    pub write_batches: u64,
    pub content_sha256: String,
}

/// Eight-byte big-endian slot to unsigned V0 header SSZ.
pub const HEADERS: &str = "V0OLHeaderBySlot";
/// Block ID to original signed V0 block SSZ.
pub const BLOCKS: &str = "V0OLBlockById";
/// Block ID to the original V0 write batch.
pub const BATCHES: &str = "V0OLWriteBatchById";
/// Block ID to the ordered V0 RPC transaction array.
pub const TXS: &str = "V0OLTransactionsById";
/// Block ID to the V0 RPC account changes.
pub const CHANGES: &str = "V0OLAccountChangesById";
/// Completion metadata, published last and unpublished first.
pub const META: &str = "V0OLArchiveMeta";

/// Encodes a header index key in sled iteration order.
pub fn header_key(slot: Slot) -> [u8; 8] {
    slot.to_be_bytes()
}

/// Returns the expected row count, excluding genesis for write batches and changes.
pub fn expected_rows(tree: &str, terminal_slot: Slot, blocks: u64) -> u64 {
    if matches!(tree, BATCHES | CHANGES) {
        terminal_slot
    } else {
        blocks
    }
}
/// Canonical tree order for the archive digest.
pub const DATA: [&str; 5] = [HEADERS, BLOCKS, BATCHES, TXS, CHANGES];

fn field(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_be_bytes());
    hash.update(bytes);
}

/// Hashes the domain, then each tree name, big-endian u64 row count and sorted key/value pairs.
/// The domain, names, keys and values each have a big-endian u64 byte-length prefix.
/// The caller must ensure the data trees already exist and have no concurrent writers.
pub fn digest(db: &sled::Db) -> sled::Result<String> {
    let mut hash = Sha256::new();
    field(&mut hash, b"strata/v0-sled-archive/1");
    for name in DATA {
        let tree = db.open_tree(name)?;
        field(&mut hash, name.as_bytes());
        hash.update((tree.len() as u64).to_be_bytes());
        for row in tree.iter() {
            let (key, value) = row?;
            field(&mut hash, &key);
            field(&mut hash, &value);
        }
    }
    Ok(format!("{:x}", hash.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_match_the_cross_release_wire_format() {
        assert_eq!(header_key(258), [0, 0, 0, 0, 0, 0, 1, 2]);
        assert_eq!(&COMPLETE_KEY, b"complete");
        assert_eq!(DATA.map(|name| expected_rows(name, 2, 3)), [3, 3, 2, 3, 2]);
    }
}
