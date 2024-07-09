use arbitrary::Arbitrary;
use bitcoin::consensus::Encodable;
use bitcoin::hashes::Hash;
use bitcoin::Transaction;
use bitcoin::{consensus::serialize, Block};
use borsh::{BorshDeserialize, BorshSerialize};

use crate::buf::Buf32;

/// Reference to a transaction in a block.  This is the block index and the
/// position of the transaction in the block.
#[derive(Copy, Clone, Debug, Hash, Eq, PartialEq, Ord, PartialOrd)]
pub struct L1TxRef(u64, u32);

impl Into<(u64, u32)> for L1TxRef {
    fn into(self) -> (u64, u32) {
        (self.0, self.1)
    }
}

impl From<(u64, u32)> for L1TxRef {
    fn from(value: (u64, u32)) -> Self {
        Self(value.0, value.1)
    }
}

/// TODO: This is duplicate with alpen_state::l1::L1TxProof
/// Merkle proof for a TXID within a block.
// TODO rework this, make it possible to generate proofs, etc.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize, Arbitrary)]
pub struct L1TxProof {
    position: u32,
    cohashes: Vec<Buf32>,
}

impl L1TxProof {
    pub fn new(position: u32, cohashes: Vec<Buf32>) -> Self {
        Self { position, cohashes }
    }

    pub fn cohashes(&self) -> &[Buf32] {
        &self.cohashes
    }

    pub fn position(&self) -> u32 {
        self.position
    }
}

/// Tx body with a proof.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize, Arbitrary)]
pub struct L1Tx {
    proof: L1TxProof,
    tx: Vec<u8>,
}

impl L1Tx {
    pub fn new(proof: L1TxProof, tx: Vec<u8>) -> Self {
        Self { proof, tx }
    }

    pub fn proof(&self) -> &L1TxProof {
        &self.proof
    }

    pub fn tx_data(&self) -> &[u8] {
        &self.tx
    }
}

/// Describes an L1 block and associated data that we need to keep around.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize, Arbitrary)]
pub struct L1BlockManifest {
    /// Block hash/ID, kept here so we don't have to be aware of the hash function
    /// here.  This is what we use in the MMR.
    blockid: Buf32,

    /// Block header and whatever additional data we might want to query.
    header: Vec<u8>,

    /// Merkle root for the transactions in the block.  For Bitcoin, this is
    /// actually the witness transactions root, since we care about the witness
    /// data.
    txs_root: Buf32,
}

impl L1BlockManifest {
    pub fn new(blockid: Buf32, header: Vec<u8>, txs_root: Buf32) -> Self {
        Self {
            blockid,
            header,
            txs_root,
        }
    }
    pub fn block_hash(&self) -> Buf32 {
        self.blockid
    }

    pub fn txs_root(&self) -> Buf32 {
        self.txs_root
    }
}

impl From<Block> for L1BlockManifest {
    fn from(block: Block) -> Self {
        let blockid = Buf32(block.block_hash().to_raw_hash().to_byte_array().into());
        let root = block
            .witness_root()
            .map(|x| x.to_byte_array())
            .unwrap_or_default();
        let header = serialize(&block.header);
        Self {
            blockid,
            txs_root: Buf32(root.into()),
            header,
        }
    }
}

#[derive(Debug, Clone, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct TxnWithStatus {
    pub txid: Buf32,
    pub txn_raw: Vec<u8>,
    pub status: BitcoinTxnStatus,
}

impl TxnWithStatus {
    /// Create a new object corresponding a transaction sent to mempool
    pub fn new(txid: Buf32, txn_raw: Vec<u8>, status: BitcoinTxnStatus) -> Self {
        Self {
            txid,
            txn_raw,
            status,
        }
    }

    /// Create a new object corresponding a transaction sent to mempool
    pub fn new_unsent(txn: Transaction) -> Self {
        let txid = Buf32(txn.compute_txid().as_byte_array().into());
        let txn_raw = serialize(&txn);
        Self::new(txid, txn_raw, BitcoinTxnStatus::Unsent)
    }

    pub fn txid(&self) -> &Buf32 {
        &self.txid
    }

    pub fn txn_raw(&self) -> &[u8] {
        &self.txn_raw
    }

    pub fn status(&self) -> &BitcoinTxnStatus {
        &self.status
    }
}

#[derive(Debug, Clone, PartialEq, BorshSerialize, BorshDeserialize)]
pub enum BitcoinTxnStatus {
    Unsent,
    InMempool,
    Confirmed,
    Finalized,
}
