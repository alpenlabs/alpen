//! Format 1 of the cross-release V0 archive. Keys and values are hashed in sled key order.
use sha2::{Digest, Sha256};

pub(super) const HEADERS: &str = "V0OLHeaderBySlot";
pub(super) const BLOCKS: &str = "V0OLBlockById";
pub(super) const BATCHES: &str = "V0OLWriteBatchById";
pub(super) const TXS: &str = "V0OLTransactionsById";
pub(super) const CHANGES: &str = "V0OLAccountChangesById";
pub(super) const META: &str = "V0OLArchiveMeta";
pub(super) const DATA: [&str; 5] = [HEADERS, BLOCKS, BATCHES, TXS, CHANGES];

fn field(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_be_bytes());
    hash.update(bytes);
}

pub(super) fn digest(db: &sled::Db) -> sled::Result<String> {
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
