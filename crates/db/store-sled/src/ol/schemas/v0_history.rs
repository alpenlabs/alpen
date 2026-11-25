//! Format 1 of the V0 explorer archive, shared by the current node and the v0.3.5 importer.
//! The header and transaction trees cover every canonical slot from genesis through the
//! promotion terminal. The batch and account-change trees omit genesis only.
//! An importer flushes all five data trees and verifies them against the source before
//! publishing [`CompleteRecord`] under [`COMPLETE_KEY`]. Readers ignore unpublished trees.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sled::IVec;
use strata_identifiers::{Epoch, OLBlockId, Slot};
use typed_sled::{CodecError, CodecResult, ValueCodec};

use crate::{define_table_without_codec, impl_codec_key_codec};

/// Supported on-disk format.
pub const FORMAT_VERSION: u32 = 1;
type MarkerKey = [u8; 8];
/// ASCII key of the single completion record.
pub const COMPLETE_KEY: [u8; 8] = *b"complete";

/// UTF-8 JSON value published after verification. Block IDs use their hex JSON representation.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CompleteRecord {
    pub format_version: u32,
    pub source: String,
    pub decoder_revision: String,
    pub genesis_id: OLBlockId,
    pub terminal_slot: Slot,
    pub terminal_epoch: Epoch,
    pub terminal_id: OLBlockId,
    pub blocks: u64,
    pub write_batches: u64,
    pub content_sha256: String,
}

// Values from the old release are stored verbatim. Their SSZ or JSON decoder belongs to
// the consumer, so the storage codec adds no framing to the bytes.
macro_rules! data_tree {
    ($(#[$doc:meta])+ $name:ident, $key:ty, $constant:ident) => {
        define_table_without_codec!($(#[$doc])+ ($name) $key => IVec);
        impl ValueCodec<$name> for IVec {
            type Decoded = Self;
            fn encode_value(&self) -> CodecResult<Vec<u8>> { Ok(self.to_vec()) }
            fn decode_value(value: IVec) -> CodecResult<Self> { Ok(value) }
        }
        $(#[$doc])+
        pub const $constant: &str = $name::tree_name();
    };
}

data_tree!(
    /// Eight-byte big-endian slot to unsigned V0 header SSZ.
    V0OLHeaderBySlot,
    Slot,
    HEADERS
);
data_tree!(
    /// Raw 32-byte block ID to original signed V0 block SSZ.
    V0OLBlockById,
    OLBlockId,
    BLOCKS
);
data_tree!(
    /// Raw block ID to the original V0 write batch.
    V0OLWriteBatchById,
    OLBlockId,
    BATCHES
);
data_tree!(
    /// Raw block ID to the ordered array of V0 RPC transactions in UTF-8 JSON.
    V0OLTransactionsById,
    OLBlockId,
    TXS
);
data_tree!(
    /// Raw block ID to the V0 RPC account changes in UTF-8 JSON.
    V0OLAccountChangesById,
    OLBlockId,
    CHANGES
);
impl_codec_key_codec!(V0OLBlockById, OLBlockId);
impl_codec_key_codec!(V0OLWriteBatchById, OLBlockId);
impl_codec_key_codec!(V0OLTransactionsById, OLBlockId);
impl_codec_key_codec!(V0OLAccountChangesById, OLBlockId);

define_table_without_codec!(
    /// Completion record keyed by the literal eight bytes of `complete`.
    (V0OLArchiveMeta) MarkerKey => CompleteRecord
);
impl_codec_key_codec!(V0OLArchiveMeta, MarkerKey);
impl ValueCodec<V0OLArchiveMeta> for CompleteRecord {
    type Decoded = Self;
    fn encode_value(&self) -> CodecResult<Vec<u8>> {
        serde_json::to_vec(self).map_err(|source| CodecError::SerializationFailed {
            schema: META,
            source: source.into(),
        })
    }
    fn decode_value(value: IVec) -> CodecResult<Self> {
        serde_json::from_slice(&value).map_err(|source| CodecError::DeserializationFailed {
            schema: META,
            source: source.into(),
        })
    }
}

/// Metadata tree containing the completion record.
pub const META: &str = V0OLArchiveMeta::tree_name();
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
    use strata_identifiers::Buf32;
    use typed_sled::KeyCodec;

    use super::*;

    #[test]
    fn keys_match_the_cross_release_wire_format() {
        let slot = 258;
        assert_eq!(
            <Slot as KeyCodec<V0OLHeaderBySlot>>::encode_key(&slot).unwrap(),
            [0, 0, 0, 0, 0, 0, 1, 2]
        );
        let id = OLBlockId::from(Buf32::from([0xa5; 32]));
        assert_eq!(
            <OLBlockId as KeyCodec<V0OLBlockById>>::encode_key(&id).unwrap(),
            [0xa5; 32]
        );
        assert_eq!(
            <MarkerKey as KeyCodec<V0OLArchiveMeta>>::encode_key(&COMPLETE_KEY).unwrap(),
            b"complete"
        );
    }
}
