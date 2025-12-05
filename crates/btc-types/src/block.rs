use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_identifiers::{Buf32, Epoch, L1BlockCommitment, L1BlockId};

use crate::{L1HeaderRecord, L1Tx};

/// Reference to a Bitcoin transaction by block ID and transaction index.
#[derive(
    Clone, Debug, PartialEq, Eq, Arbitrary, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
pub struct L1TxRef(L1BlockId, u32);

impl L1TxRef {
    pub fn new(blkid: L1BlockId, idx: u32) -> Self {
        Self(blkid, idx)
    }

    pub fn blkid(&self) -> &L1BlockId {
        &self.0
    }

    pub fn idx(&self) -> u32 {
        self.1
    }
}

impl From<L1TxRef> for (L1BlockId, u32) {
    fn from(val: L1TxRef) -> Self {
        (val.0, val.1)
    }
}

impl From<(L1BlockId, u32)> for L1TxRef {
    fn from(val: (L1BlockId, u32)) -> Self {
        Self::new(val.0, val.1)
    }
}

impl From<(&L1BlockId, u32)> for L1TxRef {
    fn from(val: (&L1BlockId, u32)) -> Self {
        Self::new(*val.0, val.1)
    }
}

/// Bitcoin-anchored block manifest containing header record and transactions.
#[derive(
    Clone, Debug, PartialEq, Eq, Arbitrary, BorshSerialize, BorshDeserialize, Deserialize, Serialize,
)]
pub struct L1BlockManifest {
    /// The actual l1 record
    record: L1HeaderRecord,

    /// List of interesting transactions we took out.
    txs: Vec<L1Tx>,

    /// Epoch, which was used to generate this manifest.
    epoch: Epoch,

    /// Block height.
    height: u64,
}

impl L1BlockManifest {
    pub fn new(record: L1HeaderRecord, txs: Vec<L1Tx>, epoch: Epoch, height: u64) -> Self {
        Self {
            record,
            txs,
            epoch,
            height,
        }
    }

    pub fn record(&self) -> &L1HeaderRecord {
        &self.record
    }

    pub fn txs(&self) -> &[L1Tx] {
        &self.txs
    }

    pub fn txs_vec(&self) -> &Vec<L1Tx> {
        &self.txs
    }

    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub fn blkid(&self) -> &L1BlockId {
        &self.record.blkid
    }

    #[deprecated(note = "use .blkid()")]
    pub fn block_hash(&self) -> L1BlockId {
        *self.record.blkid()
    }

    pub fn height(&self) -> u64 {
        self.height
    }

    pub fn header(&self) -> &[u8] {
        self.record.buf()
    }

    pub fn txs_root(&self) -> Buf32 {
        *self.record.wtxs_root()
    }

    pub fn get_prev_blockid(&self) -> L1BlockId {
        self.record().parent_blkid()
    }

    pub fn into_record(self) -> L1HeaderRecord {
        self.record
    }
}

impl From<L1BlockManifest> for L1BlockCommitment {
    fn from(value: L1BlockManifest) -> Self {
        Self::from_height_u64(value.height(), *value.blkid()).expect("height should be valid")
    }
}

impl From<&L1BlockManifest> for L1BlockCommitment {
    fn from(value: &L1BlockManifest) -> Self {
        Self::from_height_u64(value.height(), *value.blkid()).expect("height should be valid")
    }
}
