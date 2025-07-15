//! Utility functions to work with cryptographic properties of Bitcoin blocks.
//!
//! Its function is to compute and verify various primitives including Merkle roots,
//! witness commitments, and proof-of-work validation. These functions are designed
//! to be equivalent to the corresponding methods found in the
//! [`bitcoin`](bitcoin::Block), providing custom implementations where necessary.

use bitcoin::{
    block::Header, consensus::Encodable, hashes::Hash, Block, BlockHash, Transaction, TxMerkleNode,
    WitnessCommitment, WitnessMerkleNode,
};
use strata_primitives::{
    buf::Buf32,
    hash::sha256d,
    l1::{compute_block_hash, L1TxProof},
};

use crate::{
    merkle::calculate_root,
    tx::{compute_txid, compute_wtxid},
};

/// Computes the transaction merkle root.
///
/// Equivalent to [`compute_merkle_root`](Block::compute_merkle_root)
pub fn compute_merkle_root(block: &Block) -> Option<Buf32> {
    let hashes = block.txdata.iter().map(compute_txid);
    calculate_root(hashes)
}

/// Computes the witness root.
///
/// Equivalent to [`witness_root`](Block::witness_root)
pub fn compute_witness_root(transactions: &[Transaction]) -> Option<WitnessMerkleNode> {
    let hashes = transactions.iter().enumerate().map(|(i, t)| {
        if i == 0 {
            // Replace the first hash with zeroes.
            Buf32::zero()
        } else {
            compute_wtxid(t)
        }
    });
    calculate_root(hashes).map(|root| WitnessMerkleNode::from_byte_array(root.0))
}

/// Checks if Merkle root of header matches Merkle root of the transaction list.
///
/// Equivalent to [`check_merkle_root`](Block::check_merkle_root).
pub fn check_merkle_root(block: &Block) -> bool {
    match compute_merkle_root(block) {
        Some(merkle_root) => {
            block.header.merkle_root == TxMerkleNode::from_byte_array(*merkle_root.as_ref())
        }
        None => false,
    }
}

/// Scans the given coinbase transaction for a witness commitment and returns it if found.
///
/// This function iterates over the outputs of the provided `coinbase` transaction from the end
/// towards the beginning, looking for an output whose `script_pubkey` starts with the "magic" bytes
/// `[0x6a, 0x24, 0xaa, 0x21, 0xa9, 0xed]`. This pattern indicates an `OP_RETURN` with an
/// embedded witness commitment header. If such an output is found, the function extracts the
/// following 32 bytes as the witness commitment and returns a [`WitnessCommitment`].
///
/// Based on: [rust-bitcoin](https://github.com/rust-bitcoin/rust-bitcoin/blob/b97be3d4974d40cf348b280718d1367b8148d1ba/bitcoin/src/blockdata/block.rs#L190-L210).
pub fn witness_commitment_from_coinbase(coinbase: &Transaction) -> Option<WitnessCommitment> {
    // Consists of OP_RETURN, OP_PUSHBYTES_36, and four "witness header" bytes.
    const MAGIC: [u8; 6] = [0x6a, 0x24, 0xaa, 0x21, 0xa9, 0xed];

    // Commitment is in the last output that starts with magic bytes.
    if let Some(pos) = coinbase
        .output
        .iter()
        .rposition(|o| o.script_pubkey.len() >= 38 && o.script_pubkey.as_bytes()[0..6] == MAGIC)
    {
        let bytes =
            <[u8; 32]>::try_from(&coinbase.output[pos].script_pubkey.as_bytes()[6..38]).unwrap();
        Some(WitnessCommitment::from_byte_array(bytes))
    } else {
        None
    }
}

/// Computes the witness commitment for the block's transaction list.
///
/// Equivalent to [`compute_witness_commitment`](Block::compute_witness_commitment).
pub fn compute_witness_commitment(
    transactions: &[Transaction],
    witness_reserved_value: &[u8],
) -> Option<WitnessCommitment> {
    compute_witness_root(transactions).map(|witness_root| {
        let mut vec = vec![];
        witness_root
            .consensus_encode(&mut vec)
            .expect("engines don't error");
        vec.extend(witness_reserved_value);
        WitnessCommitment::from_byte_array(*sha256d(&vec).as_ref())
    })
}

/// Checks the integrity of a block using the provided coinbase inclusion proof.
///
/// We pass the `inclusion_proof` for the coinbase transaction to avoid recalculating
/// the entire Merkle root for verifying coinbase inclusion. This optimization
/// simplifies the verification logic and improves performance, for blocks containing SegWit
/// transactions.
///
/// This function applies different validation paths depending on whether the block
/// includes segwit transactions:
///
/// 1. **Blocks with segwit transactions**
///    - Verifies that the witness commitment in the coinbase transaction matches the aggregated
///      witness data of the block’s segwit transactions.
///    - Checks the coinbase transaction's inclusion in the block's Merkle tree using the provided
///      `inclusion_proof`.
///
/// 2. **Blocks without segwit transactions**
///    - Validates the Merkle root by comparing the block header's Merkle root with the Merkle root
///      computed from all transactions.
///
/// # Returns
///
/// * `true` if all integrity checks pass.
/// * `false` otherwise.
pub fn check_block_integrity(block: &Block, coinbase_inclusion_proof: &Option<L1TxProof>) -> bool {
    let Block { header, txdata } = block;
    if txdata.is_empty() {
        return false;
    }

    let coinbase = &txdata[0];
    if !coinbase.is_coinbase() {
        return false;
    }

    if let Some(commitment) = witness_commitment_from_coinbase(coinbase) {
        // If we have a witness commitment, we also need an inclusion proof.
        let proof = match coinbase_inclusion_proof {
            Some(proof) => proof,
            None => return false,
        };

        // Gather the witness data; it must have exactly one element of length 32 bytes.
        let witness_vec: Vec<_> = coinbase.input[0].witness.iter().collect();
        if witness_vec.len() != 1 || witness_vec[0].len() != 32 {
            return false;
        }

        // Check that the computed witness commitment matches and that the proof is valid.
        let is_valid_commitment = compute_witness_commitment(txdata, witness_vec[0])
            .is_some_and(|value| commitment == value);

        let is_valid_inclusion = proof.verify(coinbase, header.merkle_root.to_byte_array().into());

        is_valid_commitment && is_valid_inclusion
    } else {
        // If there's no witness commitment at all, fall back to a merkle root check.
        check_merkle_root(block)
    }
}

/// Checks that the proof-of-work for the block is valid.
pub fn check_pow(block: &Header) -> bool {
    let target = block.target();
    let block_hash = BlockHash::from_byte_array(*compute_block_hash(block).as_ref());
    target.is_met_by(block_hash)
}

#[cfg(test)]
mod tests {
    use bitcoin::Witness;
    use strata_test_utils_btc::segment::BtcChainSegment;

    use super::*;

    #[test]
    fn test_block_with_valid_witness() {
        let block = BtcChainSegment::load_full_block();
        let coinbase_inclusion_proof = L1TxProof::generate(&block.txdata, 0);
        assert!(check_block_integrity(
            &block,
            &Some(coinbase_inclusion_proof)
        ));
    }

    #[test]
    fn test_block_with_invalid_coinbase_inclusion_proof() {
        let block = BtcChainSegment::load_full_block();
        assert!(!check_block_integrity(&block, &None));
    }

    #[test]
    fn test_block_with_valid_inclusion_proof_of_other_tx() {
        let block = BtcChainSegment::load_full_block();
        let non_coinbase_inclusion_proof = L1TxProof::generate(&block.txdata, 1);
        assert!(!check_block_integrity(
            &block,
            &Some(non_coinbase_inclusion_proof)
        ));
    }

    #[test]
    fn test_block_with_witness_removed() {
        let mut block = BtcChainSegment::load_full_block();
        let empty_witness = Witness::new();

        // Remove witness data from all transactions.
        for tx in &mut block.txdata {
            for input in &mut tx.input {
                input.witness = empty_witness.clone();
            }
        }

        assert!(!check_block_integrity(&block, &None));
    }

    #[test]
    fn test_block_with_removed_witness_but_valid_inclusion_proof() {
        let mut block = BtcChainSegment::load_full_block();
        let empty_witness = Witness::new();

        // Remove witness data from all transactions.
        for tx in &mut block.txdata {
            for input in &mut tx.input {
                input.witness = empty_witness.clone();
            }
        }

        let valid_inclusion_proof = L1TxProof::generate(&block.txdata, 0);
        assert!(!check_block_integrity(&block, &Some(valid_inclusion_proof)));
    }

    #[test]
    fn test_block_without_witness_data() {
        let btc_chain = BtcChainSegment::load();
        let block = btc_chain.get_block_at(40321).unwrap();

        // Verify with an empty inclusion proof.
        assert!(check_block_integrity(&block, &None));

        // Verify with a valid inclusion proof.
        let valid_inclusion_proof = L1TxProof::generate(&block.txdata, 0);
        assert!(check_block_integrity(&block, &Some(valid_inclusion_proof)));
    }

    #[test]
    fn test_proof_of_work() {
        let block = BtcChainSegment::load_full_block();

        // Validate the block's proof-of-work.
        assert!(block.header.validate_pow(block.header.target()).is_ok());
        assert!(check_pow(&block.header));
    }
}
