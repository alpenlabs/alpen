//! DA correctness witnesses for the acct proof.

use rkyv::{Archive, Deserialize, Serialize};
use rkyv_impl::archive_impl;

use crate::dedup_witness::{ArchivedDedupWitness, DedupWitness};

/// Top-level DA witness bundle for one EE batch.
#[derive(Clone, Debug, Default, Archive, Deserialize, Serialize)]
#[rkyv(derive(Debug))]
pub struct DaWitness {
    /// One per L1 block holding DA commit/reveal transactions for this batch.
    blocks: Vec<DaBlockWitness>,

    /// Supplementary witness resolving blob references not carried inline.
    dedup_da_witness: DedupWitness,
}

impl DaWitness {
    pub fn new(blocks: Vec<DaBlockWitness>, dedup_da_witness: DedupWitness) -> Self {
        Self {
            blocks,
            dedup_da_witness,
        }
    }

    pub fn empty() -> Self {
        Self::default()
    }

    pub fn blocks(&self) -> &[DaBlockWitness] {
        &self.blocks
    }

    pub fn dedup_da_witness(&self) -> &DedupWitness {
        &self.dedup_da_witness
    }
}

impl ArchivedDaWitness {
    pub fn blocks(&self) -> &[ArchivedDaBlockWitness] {
        &self.blocks
    }

    pub fn dedup_da_witness(&self) -> &ArchivedDedupWitness {
        &self.dedup_da_witness
    }
}

/// Block-level public L1 reference data used for DA transaction inclusion.
///
/// This mirrors the reduced L1 block ref shape without using the existing
/// identifier wrapper types because the witness crosses the rkyv private-input
/// boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Archive, Deserialize, Serialize)]
#[rkyv(derive(Debug))]
pub struct L1DaBlockInclusion {
    /// Bitcoin block height.
    l1_block_height: u32,

    /// Bitcoin block hash in internal byte order.
    l1_block_hash: [u8; 32],

    /// Witness transaction ID Merkle root in internal byte order.
    wtxids_root: [u8; 32],
}

impl L1DaBlockInclusion {
    pub fn new(l1_block_height: u32, l1_block_hash: [u8; 32], wtxids_root: [u8; 32]) -> Self {
        Self {
            l1_block_height,
            l1_block_hash,
            wtxids_root,
        }
    }

    pub fn l1_block_height(&self) -> u32 {
        self.l1_block_height
    }

    pub fn l1_block_hash(&self) -> &[u8; 32] {
        &self.l1_block_hash
    }

    pub fn wtxids_root(&self) -> &[u8; 32] {
        &self.wtxids_root
    }
}

impl ArchivedL1DaBlockInclusion {
    pub fn l1_block_height(&self) -> u32 {
        self.l1_block_height.into()
    }

    pub fn l1_block_hash(&self) -> &[u8; 32] {
        &self.l1_block_hash
    }

    pub fn wtxids_root(&self) -> &[u8; 32] {
        &self.wtxids_root
    }
}

/// Witness data for one L1 block that contains DA transactions.
#[derive(Clone, Debug, Archive, Deserialize, Serialize)]
#[rkyv(derive(Debug))]
pub struct DaBlockWitness {
    /// L1 block inclusion target committed through public ledger refs.
    inclusion: L1DaBlockInclusion,

    /// DA transactions in this L1 block.
    txs: Vec<DaTxWitness>,
}

impl DaBlockWitness {
    pub fn new(inclusion: L1DaBlockInclusion, txs: Vec<DaTxWitness>) -> Self {
        Self { inclusion, txs }
    }

    pub fn inclusion(&self) -> &L1DaBlockInclusion {
        &self.inclusion
    }

    pub fn txs(&self) -> &[DaTxWitness] {
        &self.txs
    }
}

impl ArchivedDaBlockWitness {
    pub fn inclusion(&self) -> &ArchivedL1DaBlockInclusion {
        &self.inclusion
    }

    pub fn txs(&self) -> &[ArchivedDaTxWitness] {
        &self.txs
    }
}

/// Witness data for a single DA transaction.
#[derive(Clone, Debug, Archive, Deserialize, Serialize)]
#[rkyv(derive(Debug))]
pub struct DaTxWitness {
    /// Raw consensus-encoded Bitcoin transaction bytes.
    raw_tx: Vec<u8>,

    /// Merkle proof from this transaction's wtxid to the block's wtxids root.
    wtxid_inclusion_proof: BitcoinMerkleProof,
}

impl DaTxWitness {
    pub fn new(raw_tx: Vec<u8>, wtxid_inclusion_proof: BitcoinMerkleProof) -> Self {
        Self {
            raw_tx,
            wtxid_inclusion_proof,
        }
    }

    pub fn wtxid_inclusion_proof(&self) -> &BitcoinMerkleProof {
        &self.wtxid_inclusion_proof
    }
}

#[archive_impl]
impl DaTxWitness {
    pub fn raw_tx(&self) -> &[u8] {
        &self.raw_tx
    }
}

impl ArchivedDaTxWitness {
    pub fn wtxid_inclusion_proof(&self) -> &ArchivedBitcoinMerkleProof {
        &self.wtxid_inclusion_proof
    }
}

/// Bitcoin Merkle inclusion proof.
///
/// `siblings` is ordered leaf-first. `position` is the leaf index in the
/// bottom layer; bit `i` selects whether `siblings[i]` is on the left or right
/// of the running hash at level `i`.
#[derive(Clone, Debug, Default, Archive, Deserialize, Serialize)]
#[rkyv(derive(Debug))]
pub struct BitcoinMerkleProof {
    siblings: Vec<[u8; 32]>,
    position: u32,
}

impl BitcoinMerkleProof {
    pub fn new(siblings: Vec<[u8; 32]>, position: u32) -> Self {
        Self { siblings, position }
    }

    pub fn position(&self) -> u32 {
        self.position
    }
}

#[archive_impl]
impl BitcoinMerkleProof {
    pub fn siblings(&self) -> &[[u8; 32]] {
        &self.siblings
    }
}

impl ArchivedBitcoinMerkleProof {
    pub fn position(&self) -> u32 {
        self.position.into()
    }
}

#[cfg(test)]
mod tests {
    use rkyv::rancor::Error as RkyvError;

    use super::*;
    use crate::dedup_witness::BytecodePreimage;

    #[test]
    fn da_witness_empty_roundtrips_through_rkyv() {
        let witness = DaWitness::empty();

        let bytes = rkyv::to_bytes::<RkyvError>(&witness).unwrap();
        let archived = rkyv::access::<ArchivedDaWitness, RkyvError>(&bytes).unwrap();

        assert!(archived.blocks().is_empty());
        assert!(archived
            .dedup_da_witness()
            .deduped_bytecode_preimages()
            .is_empty());
    }

    #[test]
    fn da_witness_with_one_block_roundtrips_through_rkyv() {
        let inclusion = L1DaBlockInclusion::new(42, [0x11; 32], [0x22; 32]);
        let proof = BitcoinMerkleProof::new(vec![[0x33; 32]], 7);
        let tx = DaTxWitness::new(vec![0x44, 0x55], proof);
        let block = DaBlockWitness::new(inclusion, vec![tx]);
        let witness = DaWitness::new(vec![block], DedupWitness::empty());

        let bytes = rkyv::to_bytes::<RkyvError>(&witness).unwrap();
        let archived = rkyv::access::<ArchivedDaWitness, RkyvError>(&bytes).unwrap();

        assert_eq!(archived.blocks().len(), 1);
        let block = &archived.blocks()[0];
        assert_eq!(block.inclusion().l1_block_height(), 42);
        assert_eq!(block.inclusion().l1_block_hash(), &[0x11; 32]);
        assert_eq!(block.inclusion().wtxids_root(), &[0x22; 32]);
        assert_eq!(block.txs().len(), 1);
        let tx = &block.txs()[0];
        assert_eq!(tx.raw_tx(), &[0x44, 0x55]);
        assert_eq!(tx.wtxid_inclusion_proof().siblings(), &[[0x33; 32]]);
        assert_eq!(tx.wtxid_inclusion_proof().position(), 7);
    }

    #[test]
    fn da_witness_with_known_bytecode_roundtrips_through_rkyv() {
        let preimage = BytecodePreimage::new(vec![0x60, 0x80]);
        let witness = DaWitness::new(Vec::new(), DedupWitness::new(vec![preimage]));

        let bytes = rkyv::to_bytes::<RkyvError>(&witness).unwrap();
        let archived = rkyv::access::<ArchivedDaWitness, RkyvError>(&bytes).unwrap();

        assert!(archived.blocks().is_empty());
        let preimages = archived.dedup_da_witness().deduped_bytecode_preimages();
        assert_eq!(preimages.len(), 1);
        assert_eq!(preimages[0].bytecode(), &[0x60, 0x80]);
    }
}
