use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use super::L1BlockId;
use crate::{buf::Buf32, hash};

/// Header and the wtxs root.
///
/// This is the core data we need to make proof against a L1 block.  We could
/// omit the wtxs root, but we'd need to re-prove it every time, and that would
/// waste space.  So we treat this like you would an "extended header" or
/// something.
#[derive(Clone, Debug, Eq, PartialEq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct L1HeaderRecord {
    /// L1 block ID here so that we don't have to recompute it too much, which
    /// is expensive in proofs.
    pub(crate) blkid: L1BlockId,

    /// Serialized header.  For Bitcoin this is always 80 bytes.
    pub(crate) buf: Vec<u8>,

    /// Root of the transaction witnesses tree.
    ///
    /// This is how we check envelopes, since those are only present in the
    /// witness transaction serialization.
    pub(crate) wtxs_root: Buf32,
}

impl L1HeaderRecord {
    pub fn new(blkid: L1BlockId, buf: Vec<u8>, wtxs_root: Buf32) -> Self {
        Self {
            blkid,
            buf,
            wtxs_root,
        }
    }

    /// Creates a new instance serialized header and the wtxs root.
    pub fn create_from_serialized_header(buf: Vec<u8>, wtxs_root: Buf32) -> Self {
        assert_eq!(buf.len(), 80, "l1: header record not 80 bytes");
        let blkid = hash::sha256d(&buf).into();
        Self::new(blkid, buf, wtxs_root)
    }

    pub fn blkid(&self) -> &L1BlockId {
        &self.blkid
    }

    #[allow(clippy::missing_const_for_fn)]
    pub fn buf(&self) -> &[u8] {
        &self.buf
    }

    pub fn wtxs_root(&self) -> &Buf32 {
        &self.wtxs_root
    }

    /// Extracts the parent block ID from the header record.
    pub fn parent_blkid(&self) -> L1BlockId {
        assert_eq!(self.buf.len(), 80, "l1: header record not 80 bytes");
        let mut buf = [0; 32];
        buf.copy_from_slice(&self.buf()[4..36]); // range of parent field bytes
        L1BlockId::from(Buf32::from(buf))
    }
}

impl<'a> Arbitrary<'a> for L1HeaderRecord {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        // Bitcoin headers are always 80 bytes, so we generate it like that.
        // However, we don't want to hardcode the data structure like that *just
        // in case*.
        let arr = <[u8; 80]>::arbitrary(u)?;
        Ok(Self::create_from_serialized_header(
            arr.to_vec(),
            Buf32::arbitrary(u)?,
        ))
    }
}
