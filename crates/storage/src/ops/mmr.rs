//! MMR database operation interface.

use strata_db_types::traits::MmrDatabase;
use strata_merkle::MerkleProofB32 as MerkleProof;

use crate::exec::*;

inst_ops_simple! {
    (<D: MmrDatabase> => MmrDataOps) {
        append_leaf(hash: [u8; 32]) => u64;
        generate_proof(index: u64) => MerkleProof;
        generate_proofs(start: u64, end: u64) => Vec<MerkleProof>;
        pop_leaf() => Option<[u8; 32]>;
    }
}
