//! DA correctness witnesses for the acct (outer) proof.
//!
//! Carries reveal-tx witness payloads and Bitcoin Merkle inclusion proofs
//! needed to (a) reassemble the published `DaBlob` from chunked envelope
//! reveals and (b) verify each reveal-tx wtxid is included in a Bitcoin
//! block whose header chains up to `l1_block_hash`.
//!
//! The acct guest attests inclusion *under `l1_block_hash` ancestry*, not
//! Bitcoin canonicality. The OL re-checks canonicality against its own
//! L1 Header MMR when processing the resulting `EEUpdate`.
//!
//! Bitcoin types are passed as raw bytes so the guest can avoid pulling
//! in the full `bitcoin` crate just for parsing. Hashes are 32-byte
//! arrays in internal byte order.

use rkyv::{Archive, Deserialize, Serialize, vec::ArchivedVec};
use rkyv_impl::archive_impl;

/// Top-level DA witness bundle for one batch.
#[derive(Clone, Debug, Default, Archive, Deserialize, Serialize)]
#[rkyv(derive(Debug))]
pub struct DaWitness {
    /// Public input: tip hash the proof's inclusion claims are anchored
    /// to. Bitcoin block hash, internal byte order (32 bytes).
    l1_block_hash: [u8; 32],

    /// One per L1 block holding reveals for this batch, ordered
    /// ascending by height.
    blocks: Vec<DaBlockWitness>,
}

impl DaWitness {
    pub fn new(l1_block_hash: [u8; 32], blocks: Vec<DaBlockWitness>) -> Self {
        Self {
            l1_block_hash,
            blocks,
        }
    }

    /// Returns an empty witness. The guest skips DA verification when the
    /// witness is empty, so this is the right value for tests and perf
    /// fixtures that don't exercise the DA path.
    pub fn empty() -> Self {
        Self::default()
    }
}

#[archive_impl]
impl DaWitness {
    pub fn l1_block_hash(&self) -> &[u8; 32] {
        &self.l1_block_hash
    }
}

impl DaWitness {
    pub fn blocks(&self) -> &[DaBlockWitness] {
        &self.blocks
    }
}

impl ArchivedDaWitness {
    pub fn blocks(&self) -> &[ArchivedDaBlockWitness] {
        &self.blocks
    }
}

/// Witness data for one L1 block that contains DA reveal txs.
#[derive(Clone, Debug, Archive, Deserialize, Serialize)]
#[rkyv(derive(Debug))]
pub struct DaBlockWitness {
    /// Bitcoin block header, consensus-encoded (80 bytes).
    raw_header: Vec<u8>,

    /// Headers from the block above `raw_header` up to (and including)
    /// the block whose hash is `DaWitness.l1_block_hash`. Each entry is
    /// consensus-encoded (80 bytes). Empty when `raw_header` already
    /// hashes to `l1_block_hash`.
    raw_header_chain_to_tip: Vec<Vec<u8>>,

    /// Coinbase tx, consensus-encoded. Carries the BIP-141 witness
    /// commitment in an OP_RETURN output that the guest cross-checks
    /// against the wtxid Merkle root.
    raw_coinbase_tx: Vec<u8>,

    /// Merkle proof: coinbase txid → header.merkle_root.
    coinbase_to_header_proof: BitcoinMerkleProof,

    /// Reveal txs in this block, each carrying one DA chunk in its
    /// envelope witness.
    reveals: Vec<RevealWitness>,
}

impl DaBlockWitness {
    pub fn new(
        raw_header: Vec<u8>,
        raw_header_chain_to_tip: Vec<Vec<u8>>,
        raw_coinbase_tx: Vec<u8>,
        coinbase_to_header_proof: BitcoinMerkleProof,
        reveals: Vec<RevealWitness>,
    ) -> Self {
        Self {
            raw_header,
            raw_header_chain_to_tip,
            raw_coinbase_tx,
            coinbase_to_header_proof,
            reveals,
        }
    }
}

#[archive_impl]
impl DaBlockWitness {
    pub fn raw_header(&self) -> &[u8] {
        &self.raw_header
    }

    pub fn raw_coinbase_tx(&self) -> &[u8] {
        &self.raw_coinbase_tx
    }
}

impl DaBlockWitness {
    pub fn reveals(&self) -> &[RevealWitness] {
        &self.reveals
    }

    pub fn raw_header_chain_to_tip(&self) -> &[Vec<u8>] {
        &self.raw_header_chain_to_tip
    }

    pub fn coinbase_to_header_proof(&self) -> &BitcoinMerkleProof {
        &self.coinbase_to_header_proof
    }
}

impl ArchivedDaBlockWitness {
    pub fn reveals(&self) -> &[ArchivedRevealWitness] {
        &self.reveals
    }

    pub fn raw_header_chain_to_tip(&self) -> &[ArchivedVec<u8>] {
        &self.raw_header_chain_to_tip
    }

    pub fn coinbase_to_header_proof(&self) -> &ArchivedBitcoinMerkleProof {
        &self.coinbase_to_header_proof
    }
}

/// Witness data for a single DA reveal tx.
#[derive(Clone, Debug, Archive, Deserialize, Serialize)]
#[rkyv(derive(Debug))]
pub struct RevealWitness {
    /// Reveal tx txid (32 bytes, internal byte order).
    txid: [u8; 32],

    /// Reveal tx wtxid (32 bytes, internal byte order).
    wtxid: [u8; 32],

    /// Merkle proof: reveal wtxid → witness root (committed in
    /// coinbase OP_RETURN).
    wtxid_to_witness_root_proof: BitcoinMerkleProof,

    /// Envelope payload bytes lifted from the reveal's witness:
    /// `chunk_header (37 bytes) || chunk_payload`. Same shape that
    /// `alpen_ee_common::decode_da_chunk` consumes.
    envelope_payload: Vec<u8>,
}

impl RevealWitness {
    pub fn new(
        txid: [u8; 32],
        wtxid: [u8; 32],
        wtxid_to_witness_root_proof: BitcoinMerkleProof,
        envelope_payload: Vec<u8>,
    ) -> Self {
        Self {
            txid,
            wtxid,
            wtxid_to_witness_root_proof,
            envelope_payload,
        }
    }
}

#[archive_impl]
impl RevealWitness {
    pub fn txid(&self) -> &[u8; 32] {
        &self.txid
    }

    pub fn wtxid(&self) -> &[u8; 32] {
        &self.wtxid
    }

    pub fn envelope_payload(&self) -> &[u8] {
        &self.envelope_payload
    }
}

impl RevealWitness {
    pub fn wtxid_to_witness_root_proof(&self) -> &BitcoinMerkleProof {
        &self.wtxid_to_witness_root_proof
    }
}

impl ArchivedRevealWitness {
    pub fn wtxid_to_witness_root_proof(&self) -> &ArchivedBitcoinMerkleProof {
        &self.wtxid_to_witness_root_proof
    }
}

/// Bitcoin Merkle inclusion proof.
///
/// `siblings` is the hash path from the leaf up to (but not including)
/// the root, ordered leaf-first. `position` is the leaf's index in the
/// bottom layer; bit `i` of `position` selects whether `siblings[i]`
/// concatenates to the left (bit set) or right (bit clear) of the
/// running hash at level `i`.
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
}

#[archive_impl]
impl BitcoinMerkleProof {
    pub fn siblings(&self) -> &[[u8; 32]] {
        &self.siblings
    }
}

impl BitcoinMerkleProof {
    pub fn position(&self) -> u32 {
        self.position
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

    #[test]
    fn da_witness_empty_roundtrips_through_rkyv() {
        let w = DaWitness::empty();
        let bytes = rkyv::to_bytes::<RkyvError>(&w).unwrap();
        let archived = rkyv::access::<ArchivedDaWitness, RkyvError>(&bytes).unwrap();
        assert_eq!(archived.l1_block_hash(), &[0u8; 32]);
        assert!(archived.blocks().is_empty());
    }

    #[test]
    fn da_witness_with_one_block_roundtrips_through_rkyv() {
        let reveal = RevealWitness::new(
            [0x11; 32],
            [0x22; 32],
            BitcoinMerkleProof::new(vec![[0x33; 32]], 0),
            vec![0x44, 0x55],
        );
        let block = DaBlockWitness::new(
            vec![0xAA; 80],
            vec![],
            vec![0xBB; 50],
            BitcoinMerkleProof::new(vec![[0x66; 32], [0x77; 32]], 1),
            vec![reveal],
        );
        let w = DaWitness::new([0x99; 32], vec![block]);

        let bytes = rkyv::to_bytes::<RkyvError>(&w).unwrap();
        let archived = rkyv::access::<ArchivedDaWitness, RkyvError>(&bytes).unwrap();

        assert_eq!(archived.l1_block_hash(), &[0x99; 32]);
        assert_eq!(archived.blocks().len(), 1);
        let blk = &archived.blocks()[0];
        assert_eq!(blk.raw_header(), &[0xAA; 80]);
        assert_eq!(blk.reveals().len(), 1);
        let r = &blk.reveals()[0];
        assert_eq!(r.txid(), &[0x11; 32]);
        assert_eq!(r.wtxid(), &[0x22; 32]);
        assert_eq!(r.envelope_payload(), &[0x44, 0x55]);
    }
}
