//! L1 inclusion checks for DA witnesses.

use bitcoin::{Transaction, consensus::deserialize as btc_deserialize, hashes::Hash as _};
use sha2::{Digest, Sha256};
use strata_ee_acct_runtime::{
    ArchivedBitcoinMerkleProof, ArchivedDaBlockWitness, BitcoinMerkleProof,
};
use strata_snark_acct_types::{LedgerRefs, l1_block_ref_leaf_hash};

use super::error::DaVerificationError;

/// Hashes one Bitcoin merkle-tree level as `SHA256(SHA256(left || right))`.
///
/// Bitcoin uses double-SHA-256 for wtxids and merkle-pair hashing; see
/// BIP-141 for the wtxid / witness-root commitment this verifier targets:
/// <https://github.com/bitcoin/bips/blob/master/bip-0141.mediawiki#commitment-structure>.
fn bitcoin_hash_pair(left: [u8; 32], right: [u8; 32]) -> [u8; 32] {
    let mut pair = [0u8; 64];
    pair[..32].copy_from_slice(&left);
    pair[32..].copy_from_slice(&right);

    let first = Sha256::digest(pair);
    Sha256::digest(first).into()
}

/// Computes a Bitcoin-style Merkle root from a leaf and inclusion path.
///
/// `siblings` is ordered leaf-first. `position` is the leaf index in the
/// bottom layer; bit `i` selects whether `siblings[i]` is on the left or right
/// of the running hash at level `i`.
pub fn bitcoin_merkle_root(
    leaf_hash: [u8; 32],
    siblings: &[[u8; 32]],
    mut position: u32,
) -> [u8; 32] {
    let mut root = leaf_hash;

    for sibling in siblings {
        root = if position & 1 == 0 {
            bitcoin_hash_pair(root, *sibling)
        } else {
            bitcoin_hash_pair(*sibling, root)
        };
        position >>= 1;
    }

    root
}

/// Computes the Merkle root described by a [`BitcoinMerkleProof`].
pub fn bitcoin_merkle_root_from_proof(leaf_hash: [u8; 32], proof: &BitcoinMerkleProof) -> [u8; 32] {
    bitcoin_merkle_root(leaf_hash, proof.siblings(), proof.position())
}

/// Computes the Merkle root described by an archived [`BitcoinMerkleProof`].
pub fn bitcoin_merkle_root_from_archived_proof(
    leaf_hash: [u8; 32],
    proof: &ArchivedBitcoinMerkleProof,
) -> [u8; 32] {
    bitcoin_merkle_root(leaf_hash, proof.siblings(), proof.position())
}

/// Computes the public ledger-ref entry hash for an L1 block ref.
pub(crate) fn l1_block_ref_commitment(block_hash: &[u8; 32], wtxids_root: &[u8; 32]) -> [u8; 32] {
    l1_block_ref_leaf_hash(block_hash, wtxids_root)
}

/// Verifies all witnessed DA transactions in one L1 block.
///
/// This checks both the public reduced L1 ref binding and each tx's wtxid
/// Merkle path to the block's `wtxids_root`.
pub(super) fn verify_block_witness(
    block: &ArchivedDaBlockWitness,
    ledger_refs: &LedgerRefs,
) -> Result<Vec<Transaction>, DaVerificationError> {
    verify_l1_ref_binding(block, ledger_refs)?;

    if block.txs().is_empty() {
        return Err(DaVerificationError::MissingDaTransactions);
    }

    let expected_root = *block.inclusion().wtxids_root();
    let mut decoded = Vec::with_capacity(block.txs().len());
    for tx_witness in block.txs() {
        let tx: Transaction = btc_deserialize(tx_witness.raw_tx())
            .map_err(|e| DaVerificationError::DaTxDecode(e.to_string()))?;
        let computed_root = bitcoin_merkle_root_from_archived_proof(
            tx.compute_wtxid().to_byte_array(),
            tx_witness.wtxid_inclusion_proof(),
        );
        if computed_root != expected_root {
            return Err(DaVerificationError::WtxidsRootMismatch {
                expected: expected_root,
                computed: computed_root,
            });
        }
        decoded.push(tx);
    }

    Ok(decoded)
}

/// Verifies that a witnessed DA block is claimed in public LedgerRefs.
///
/// The L1 block ref MMR leaf is indexed by L1 height and commits to
/// the SSZ tree hash of `{block_hash, wtxids_root}`.
fn verify_l1_ref_binding(
    block: &ArchivedDaBlockWitness,
    ledger_refs: &LedgerRefs,
) -> Result<(), DaVerificationError> {
    let inclusion = block.inclusion();
    let idx = u64::from(inclusion.l1_block_height());
    let expected_hash = l1_block_ref_commitment(inclusion.l1_block_hash(), inclusion.wtxids_root());

    let found = ledger_refs
        .l1_block_refs()
        .iter()
        .any(|claim| claim.idx() == idx && claim.entry_hash().as_ref() == expected_hash.as_slice());
    if !found {
        return Err(DaVerificationError::L1DaBlockRefNotInLedgerRefs { idx });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use rkyv::rancor::Error as RkyvError;

    use super::*;

    fn hex32(s: &str) -> [u8; 32] {
        assert_eq!(s.len(), 64);
        let mut out = [0u8; 32];
        for i in 0..32 {
            out[i] = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap();
        }
        out
    }

    #[test]
    fn bitcoin_merkle_root_with_empty_path_is_leaf() {
        let leaf = [0xAA; 32];

        assert_eq!(bitcoin_merkle_root(leaf, &[], 0), leaf);
    }

    #[test]
    fn bitcoin_merkle_root_respects_position_bits() {
        let left = [0x00; 32];
        let right = [0x11; 32];
        let expected = hex32("127e4900feebf53bb61ecc03d9a628da770e4f8ef65cfd6d40852cd9a553b3d5");

        assert_eq!(bitcoin_merkle_root(left, &[right], 0), expected);
        assert_eq!(bitcoin_merkle_root(right, &[left], 1), expected);
    }

    #[test]
    fn bitcoin_merkle_root_handles_multi_level_paths() {
        let leaf = [0x22; 32];
        let first_sibling = [0x22; 32];
        let left_subtree =
            hex32("127e4900feebf53bb61ecc03d9a628da770e4f8ef65cfd6d40852cd9a553b3d5");
        let expected = hex32("16c94d996b5f836f454a3e1f9be1df8ba65d708d54b8ae84f52122b9c856e928");

        assert_eq!(
            bitcoin_merkle_root(leaf, &[first_sibling, left_subtree], 2),
            expected
        );
    }

    #[test]
    fn bitcoin_merkle_root_from_proof_uses_proof_position() {
        let proof = BitcoinMerkleProof::new(vec![[0x11; 32]], 0);
        let expected = hex32("127e4900feebf53bb61ecc03d9a628da770e4f8ef65cfd6d40852cd9a553b3d5");

        assert_eq!(bitcoin_merkle_root_from_proof([0x00; 32], &proof), expected);
    }

    #[test]
    fn bitcoin_merkle_root_from_archived_proof_uses_archived_position() {
        let proof = BitcoinMerkleProof::new(vec![[0x11; 32]], 0);
        let bytes = rkyv::to_bytes::<RkyvError>(&proof).unwrap();
        let archived = rkyv::access::<ArchivedBitcoinMerkleProof, RkyvError>(&bytes).unwrap();
        let expected = hex32("127e4900feebf53bb61ecc03d9a628da770e4f8ef65cfd6d40852cd9a553b3d5");

        assert_eq!(
            bitcoin_merkle_root_from_archived_proof([0x00; 32], archived),
            expected
        );
    }
}
