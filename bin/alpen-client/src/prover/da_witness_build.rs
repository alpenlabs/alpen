//! Host-side helpers for assembling the DA witness consumed by the EE
//! outer (acct) proof.
//!
//! The Bitcoin Merkle proof shape produced here matches
//! [`alpen_acct::compute_btc_merkle_root`](strata_proofimpl_alpen_acct::compute_btc_merkle_root)
//! — same `(siblings, position)` convention so the guest can verify
//! without re-encoding.

use bitcoin::{hashes::Hash as _, Transaction};
use strata_crypto::hash::sha256d;
use strata_ee_acct_runtime::BitcoinMerkleProof;
use strata_identifiers::Buf32;

/// Builds a coinbase txid → header.merkle_root inclusion proof for the
/// transaction at index 0 (the coinbase) of `txs`.
pub(super) fn build_coinbase_inclusion_proof(txs: &[Transaction]) -> BitcoinMerkleProof {
    let leaves: Vec<[u8; 32]> = txs
        .iter()
        .map(|t| t.compute_txid().to_byte_array())
        .collect();
    build_inclusion_proof(&leaves, 0)
}

/// Builds a wtxid → witness_root inclusion proof for the transaction at
/// position `idx` within `txs`. Per BIP-141, the coinbase's wtxid leaf
/// is replaced by 32 zero bytes in the witness Merkle tree.
pub(super) fn build_wtxid_inclusion_proof(txs: &[Transaction], idx: usize) -> BitcoinMerkleProof {
    let leaves: Vec<[u8; 32]> = txs
        .iter()
        .enumerate()
        .map(|(i, t)| {
            if i == 0 {
                [0u8; 32]
            } else {
                t.compute_wtxid().to_byte_array()
            }
        })
        .collect();
    build_inclusion_proof(&leaves, idx as u32)
}

/// Builds a Bitcoin-style Merkle proof for `leaves[idx]`. Mirrors
/// Bitcoin's tree construction: at each level, an odd-length layer
/// duplicates its last node before pairwise hashing with `sha256d`.
fn build_inclusion_proof(leaves: &[[u8; 32]], idx: u32) -> BitcoinMerkleProof {
    assert!(
        (idx as usize) < leaves.len(),
        "idx {idx} out of bounds for {} leaves",
        leaves.len()
    );
    let mut cur_level = leaves.to_vec();
    let mut cur_idx = idx;
    // Pre-allocate proof depth = ceil(log2(n)).
    let depth = (usize::BITS - cur_level.len().leading_zeros()) as usize;
    let mut siblings = Vec::with_capacity(depth);

    while cur_level.len() > 1 {
        if cur_level.len() % 2 == 1 {
            cur_level.push(*cur_level.last().expect("non-empty per loop guard"));
        }
        let sibling_idx = (cur_idx ^ 1) as usize;
        siblings.push(cur_level[sibling_idx]);

        cur_level = cur_level
            .chunks(2)
            .map(|pair| {
                let mut buf = [0u8; 64];
                buf[..32].copy_from_slice(&pair[0]);
                buf[32..].copy_from_slice(&pair[1]);
                let h: Buf32 = sha256d(&buf);
                *<Buf32 as AsRef<[u8; 32]>>::as_ref(&h)
            })
            .collect();
        cur_idx >>= 1;
    }

    BitcoinMerkleProof::new(siblings, idx)
}

#[cfg(test)]
mod tests {
    use bitcoin::{
        absolute::LockTime, transaction::Version, Amount, OutPoint, ScriptBuf, Sequence,
        Transaction, TxIn, TxOut, Witness,
    };

    use super::*;

    fn dsha(buf: &[u8]) -> [u8; 32] {
        let h: Buf32 = sha256d(buf);
        *<Buf32 as AsRef<[u8; 32]>>::as_ref(&h)
    }

    fn compute_root(leaf: [u8; 32], proof: &BitcoinMerkleProof) -> [u8; 32] {
        let mut cur = leaf;
        let mut pos = proof.position();
        for sibling in proof.siblings() {
            let mut buf = [0u8; 64];
            if pos & 1 == 0 {
                buf[..32].copy_from_slice(&cur);
                buf[32..].copy_from_slice(sibling);
            } else {
                buf[..32].copy_from_slice(sibling);
                buf[32..].copy_from_slice(&cur);
            }
            cur = dsha(&buf);
            pos >>= 1;
        }
        cur
    }

    fn make_dummy_tx(nonce: u8) -> Transaction {
        Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::null(),
                script_sig: ScriptBuf::from_bytes(vec![nonce]),
                sequence: Sequence::MAX,
                witness: Witness::new(),
            }],
            output: vec![TxOut {
                value: Amount::from_sat(0),
                script_pubkey: ScriptBuf::new(),
            }],
        }
    }

    /// Bitcoin tree with an odd leaf at the end exercises the
    /// duplicate-last-node behaviour in `build_inclusion_proof`.
    fn make_dummy_txs(n: usize) -> Vec<Transaction> {
        (0..n).map(|i| make_dummy_tx(i as u8)).collect()
    }

    fn naive_merkle_root(leaves: &[[u8; 32]]) -> [u8; 32] {
        let mut cur = leaves.to_vec();
        while cur.len() > 1 {
            if cur.len() % 2 == 1 {
                cur.push(*cur.last().unwrap());
            }
            cur = cur
                .chunks(2)
                .map(|pair| {
                    let mut buf = [0u8; 64];
                    buf[..32].copy_from_slice(&pair[0]);
                    buf[32..].copy_from_slice(&pair[1]);
                    dsha(&buf)
                })
                .collect();
        }
        cur[0]
    }

    #[test]
    fn coinbase_inclusion_proof_matches_naive_root() {
        for n in [1, 2, 3, 4, 5, 8] {
            let txs = make_dummy_txs(n);
            let leaves: Vec<[u8; 32]> = txs
                .iter()
                .map(|t| t.compute_txid().to_byte_array())
                .collect();
            let expected_root = naive_merkle_root(&leaves);
            let proof = build_coinbase_inclusion_proof(&txs);
            assert_eq!(compute_root(leaves[0], &proof), expected_root, "n={n}");
        }
    }

    #[test]
    fn wtxid_inclusion_proof_matches_naive_root_with_coinbase_zeroed() {
        let txs = make_dummy_txs(5);
        let leaves: Vec<[u8; 32]> = txs
            .iter()
            .enumerate()
            .map(|(i, t)| {
                if i == 0 {
                    [0u8; 32]
                } else {
                    t.compute_wtxid().to_byte_array()
                }
            })
            .collect();
        let expected_root = naive_merkle_root(&leaves);

        // Verify proof for every non-coinbase tx.
        for (idx, leaf) in leaves.iter().enumerate().skip(1) {
            let proof = build_wtxid_inclusion_proof(&txs, idx);
            assert_eq!(compute_root(*leaf, &proof), expected_root, "idx={idx}");
        }
    }

    #[test]
    fn single_tx_proof_has_no_siblings() {
        let txs = make_dummy_txs(1);
        let proof = build_coinbase_inclusion_proof(&txs);
        assert!(proof.siblings().is_empty());
        assert_eq!(proof.position(), 0);
    }
}
