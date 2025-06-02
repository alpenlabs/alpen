use arbitrary::Arbitrary;
use bitcoin::{hashes::Hash, BlockHash};
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use super::{header_verification::HeaderVerificationState, L1HeaderRecord, L1Tx};
use crate::{buf::Buf32, hash::sha256d, impl_buf_wrapper};

/// ID of an L1 block, usually the hash of its header.
#[derive(
    Copy,
    Clone,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    Default,
    Arbitrary,
    BorshSerialize,
    BorshDeserialize,
    Deserialize,
    Serialize,
)]
pub struct L1BlockId(Buf32);

impl L1BlockId {
    /// Computes the [`L1BlockId`] from the header buf. This is expensive in proofs and
    /// should only be done when necessary.
    pub fn compute_from_header_buf(buf: &[u8]) -> L1BlockId {
        Self::from(sha256d(buf))
    }
}

impl_buf_wrapper!(L1BlockId, Buf32, 32);

impl From<BlockHash> for L1BlockId {
    fn from(value: BlockHash) -> Self {
        L1BlockId(value.into())
    }
}

impl From<L1BlockId> for BlockHash {
    fn from(value: L1BlockId) -> Self {
        BlockHash::from_byte_array(value.0.into())
    }
}

#[derive(
    Debug,
    Copy,
    Clone,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    Default,
    Arbitrary,
    BorshDeserialize,
    BorshSerialize,
    Deserialize,
    Serialize,
)]
pub struct L1BlockCommitment {
    height: u64,
    blkid: L1BlockId,
}

impl L1BlockCommitment {
    pub fn new(height: u64, blkid: L1BlockId) -> Self {
        Self { height, blkid }
    }

    pub fn height(&self) -> u64 {
        self.height
    }

    pub fn blkid(&self) -> &L1BlockId {
        &self.blkid
    }
}

/// Reference to a transaction in a block.  This is the blockid and the
/// position of the transaction in the block.
#[derive(
    Copy,
    Clone,
    Debug,
    Hash,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Arbitrary,
    BorshDeserialize,
    BorshSerialize,
    Serialize,
    Deserialize,
)]
pub struct L1TxRef(L1BlockId, u32);

impl L1TxRef {
    pub fn blk_id(&self) -> L1BlockId {
        self.0
    }

    pub fn position(&self) -> u32 {
        self.1
    }
}

impl From<L1TxRef> for (L1BlockId, u32) {
    fn from(val: L1TxRef) -> Self {
        (val.0, val.1)
    }
}

impl From<(L1BlockId, u32)> for L1TxRef {
    fn from(value: (L1BlockId, u32)) -> Self {
        Self(value.0, value.1)
    }
}

impl From<(&L1BlockId, u32)> for L1TxRef {
    fn from(value: (&L1BlockId, u32)) -> Self {
        Self(*value.0, value.1)
    }
}

/// Includes [`L1BlockManifest`] along with scan rules that it is applied to.
#[derive(
    Clone, Debug, PartialEq, Eq, Arbitrary, BorshSerialize, BorshDeserialize, Deserialize, Serialize,
)]
pub struct L1BlockManifest {
    /// The actual l1 record
    record: L1HeaderRecord,

    /// Optional header verification state
    ///
    /// For the genesis block, this field is set to `Some` containing a
    /// [HeaderVerificationState] that holds all necessary details for validating Bitcoin block
    /// headers
    /// For all subsequent blocks, this field is `None`. It is used during the initialization of
    /// the Chainstate to bootstrap the header verification process.
    // TODO: handle this properly: https://alpenlabs.atlassian.net/browse/STR-1104
    verif_state: Option<HeaderVerificationState>,

    /// List of interesting transactions we took out.
    txs: Vec<L1Tx>,

    /// Epoch, which was used to generate this manifest.
    epoch: u64,

    /// Block height.
    height: u64,
}

impl L1BlockManifest {
    pub fn new(
        record: L1HeaderRecord,
        verif_state: Option<HeaderVerificationState>,
        txs: Vec<L1Tx>,
        epoch: u64,
        height: u64,
    ) -> Self {
        Self {
            record,
            verif_state,
            txs,
            epoch,
            height,
        }
    }

    pub fn record(&self) -> &L1HeaderRecord {
        &self.record
    }

    pub fn header_verification_state(&self) -> &Option<HeaderVerificationState> {
        &self.verif_state
    }

    #[allow(clippy::missing_const_for_fn)]
    pub fn txs(&self) -> &[L1Tx] {
        &self.txs
    }

    pub fn txs_vec(&self) -> &Vec<L1Tx> {
        &self.txs
    }

    pub fn epoch(&self) -> u64 {
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
