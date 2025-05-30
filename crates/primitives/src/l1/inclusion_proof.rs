use std::marker::PhantomData;

use arbitrary::Arbitrary;
use bitcoin::Transaction;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use crate::{
    buf::Buf32,
    hash::sha256d,
    l1::{TxIdComputable, TxIdMarker, WtxIdMarker},
    utils::get_cohashes,
};

/// A generic proof structure that can handle any kind of transaction ID (e.g.,
/// [`Txid`](bitcoin::Txid) or [`Wtxid`](bitcoin::Wtxid)) by delegating the ID computation to the
/// provided type `T` that implements [`TxIdComputable`].
#[derive(
    Clone, Debug, PartialEq, Eq, Arbitrary, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
pub struct L1TxInclusionProof<T> {
    /// The 0-based position (index) of the transaction within the block's transaction list
    /// for which this proof is generated.
    position: u32,

    /// The intermediate hashes (sometimes called "siblings") needed to reconstruct the Merkle root
    /// when combined with the target transaction's own ID. These are the Merkle tree nodes at
    /// each step that pair with the current hash (either on the left or the right) to produce
    /// the next level of the tree.
    cohashes: Vec<Buf32>,

    /// A marker that preserves the association with type `T`, which implements
    /// [`TxIdComputable`]. This ensures the proof logic depends on the correct
    /// transaction ID computation ([`Txid`](bitcoin::Txid) vs.[`Wtxid`](bitcoin::Wtxid)) for the
    /// lifetime of the proof.
    _marker: PhantomData<T>,
}

impl<T> L1TxInclusionProof<T> {
    #[expect(clippy::missing_const_for_fn)]
    pub fn new(position: u32, cohashes: Vec<Buf32>) -> Self {
        Self {
            position,
            cohashes,
            _marker: PhantomData,
        }
    }

    #[allow(clippy::missing_const_for_fn)]
    pub fn cohashes(&self) -> &[Buf32] {
        &self.cohashes
    }

    pub const fn position(&self) -> u32 {
        self.position
    }
}

impl<T: TxIdComputable> L1TxInclusionProof<T> {
    /// Generates the proof for a transaction at the specified index in the list of
    /// transactions, using `T` to compute the transaction IDs.
    pub fn generate(transactions: &[Transaction], idx: u32) -> Self {
        let txids = transactions
            .iter()
            .enumerate()
            .map(|(idx, tx)| T::compute_id(tx, idx))
            .collect::<Vec<_>>();
        let (cohashes, _txroot) = get_cohashes(&txids, idx);
        L1TxInclusionProof::new(idx, cohashes)
    }

    /// Computes the merkle root for the given `transaction` using the proof's cohashes.
    pub fn compute_root(&self, transaction: &Transaction) -> Buf32 {
        // `cur_hash` represents the intermediate hash at each step. After all cohashes are
        // processed `cur_hash` becomes the root hash
        let mut cur_hash = T::compute_id(transaction, self.position as usize).0;

        let mut pos = self.position();
        for cohash in self.cohashes() {
            let mut buf = [0u8; 64];
            if pos & 1 == 0 {
                buf[0..32].copy_from_slice(&cur_hash);
                buf[32..64].copy_from_slice(cohash.as_ref());
            } else {
                buf[0..32].copy_from_slice(cohash.as_ref());
                buf[32..64].copy_from_slice(&cur_hash);
            }
            cur_hash = sha256d(&buf).0;
            pos >>= 1;
        }
        Buf32::from(cur_hash)
    }

    /// Verifies the inclusion proof of the given `transaction` against the provided merkle `root`.
    pub fn verify(&self, transaction: &Transaction, root: Buf32) -> bool {
        self.compute_root(transaction) == root
    }
}

/// Convenience type alias for the [`Txid`](bitcoin::Txid)-based proof.
pub type L1TxProof = L1TxInclusionProof<TxIdMarker>;

/// Convenience type alias for the [`Wtxid`](bitcoin::Wtxid)-based proof.
pub type L1WtxProof = L1TxInclusionProof<WtxIdMarker>;

#[cfg(test)]
mod tests {
    use bitcoin::hashes::Hash;
    use rand::{thread_rng, Rng};
    use strata_test_utils::bitcoin_mainnet_segment::BtcChainSegment;

    use super::*;
    use crate::buf::Buf32;

    #[test]
    fn test_l1_tx_proof() {
        let btc_chain = BtcChainSegment::load();
        let block = btc_chain.get_block_at(40_321).unwrap();
        let merkle_root: Buf32 = block.header.merkle_root.to_byte_array().into();
        let txs = &block.txdata;

        for (idx, tx) in txs.iter().enumerate() {
            let proof = L1TxProof::generate(txs, idx as u32);
            assert!(proof.verify(tx, merkle_root));
        }
    }

    #[test]
    fn test_l1_tx_proof_2() {
        let block = BtcChainSegment::load_full_block();
        let merkle_root: Buf32 = block.header.merkle_root.to_byte_array().into();
        let txs = &block.txdata;

        let mut rng = thread_rng();
        let idx = rng.gen_range(0..=txs.len());
        let proof = L1TxProof::generate(txs, idx as u32);
        assert!(proof.verify(&txs[idx], merkle_root));
    }

    #[test]
    fn test_l1_wtx_proof() {
        let block = BtcChainSegment::load_full_block();
        let txs = &block.txdata;
        let wtx_root = block.witness_root().unwrap().to_byte_array().into();

        let idx = 0;
        let proof = L1WtxProof::generate(txs, idx as u32);
        assert!(proof.verify(&txs[idx], wtx_root));

        let mut rng = thread_rng();
        let idx = rng.gen_range(1..=txs.len());
        let proof = L1WtxProof::generate(txs, idx as u32);
        assert!(proof.verify(&txs[idx], wtx_root));
    }
}
