use bitcoin::Block;

use crate::{L1Tx, L1TxProof, ProtocolOperation};

/// Generates an L1 transaction with proof for a given transaction index in a block.
///
/// # Parameters
/// - `block`: The block containing the transactions.
/// - `idx`: The index of the transaction within the block's transaction data.
/// - `proto_ops`: Protocol operations extracted from the transaction.
///
/// # Returns
/// - An [`L1Tx`] struct containing the proof and the serialized transaction.
///
/// # Panics
/// - If the `idx` is out of bounds for the block's transaction data.
#[cfg(feature = "bitcoin")]
pub fn generate_l1_tx(block: &Block, idx: u32, proto_ops: Vec<ProtocolOperation>) -> L1Tx {
    assert!(
        (idx as usize) < block.txdata.len(),
        "utils: tx idx out of range of block txs"
    );
    let tx = &block.txdata[idx as usize];

    let proof = L1TxProof::generate(&block.txdata, idx);

    L1Tx::new(proof, tx.clone().into(), proto_ops)
}
