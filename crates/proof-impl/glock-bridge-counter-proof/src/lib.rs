//! # Bridge Counter-Proof Crate
//!
//! This crate provides functionality to generate counter-proofs for bridge proofs.
//! Counter-proofs are used to challenge invalid bridge proofs by demonstrating either:
//!
//! - The bridge proof itself is invalid
//! - A heavier Bitcoin chain exists that contradicts the bridge proof

use std::sync::Arc;

use bitcoin::secp256k1;
use borsh::{BorshDeserialize, BorshSerialize};

mod program;
mod statement;

use statement::process_counterproof;
use zkaleido::ZkVmEnv;

/// Represents the different modes of counter-proof generation
#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
pub enum CounterproofMode {
    /// Challenge by proving the bridge proof is invalid
    InvalidBridgeProof,
    /// Challenge by providing a heavier Bitcoin chain
    HeavierChain(Vec<[u8; 80]>),
}

/// Input data required for generating a counter-proof
#[derive(Clone, Debug)]
pub struct CounterproofInput {
    /// Master public key used in the bridge proof
    pub bridge_proof_master_key: secp256k1::XOnlyPublicKey,
    /// Index of the deposit being challenged
    pub deposit_index: u32,
    /// The bridge proof transaction
    pub bridge_proof_tx: bitcoin::Transaction,
    /// Previous outputs for the bridge proof transaction
    pub bridge_proof_prevouts: Arc<[bitcoin::TxOut]>,
    /// Mode of counter-proof (invalid proof or heavier chain)
    pub mode: CounterproofMode,
}

/// Subset of [`CounterproofInput`] that is [borsh]-serializable
#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
pub(crate) struct CounterproofInputBorsh {
    bridge_proof_master_key: [u8; 32],
    deposit_index: u32,
    bridge_proof_tx_bytes: Vec<u8>,
    bridge_proof_prevouts: Vec<Vec<u8>>, // TxOut serialized as bytes
    mode: CounterproofMode,
}

impl From<CounterproofInput> for CounterproofInputBorsh {
    fn from(input: CounterproofInput) -> Self {
        use bitcoin::consensus::serialize;

        Self {
            bridge_proof_master_key: input.bridge_proof_master_key.serialize(),
            deposit_index: input.deposit_index,
            bridge_proof_tx_bytes: serialize(&input.bridge_proof_tx),
            bridge_proof_prevouts: input.bridge_proof_prevouts.iter().map(serialize).collect(),
            mode: input.mode,
        }
    }
}

/// Public outputs of the counter-proof, used for verification
#[derive(Clone, Debug, Eq, PartialEq, Hash, BorshSerialize, BorshDeserialize)]
pub struct CounterproofPublicOutput {
    /// The master public key from the challenged bridge proof
    pub bridge_proof_master_key: [u8; 32],
    /// The deposit index that was challenged
    pub deposit_index: u32,
}

/// Processes the counter-proof by reading data from the ZkVM environment,
/// validating the counter-proof conditions, and committing the result.
pub fn process_counterproof_outer(zkvm: &impl ZkVmEnv) {
    let input: CounterproofInputBorsh = zkvm.read_borsh();

    let output = process_counterproof(input).expect("Counter-proof processing failed");

    zkvm.commit_borsh(&output);
}

pub use program::{get_native_host, CounterproofProgram};

/// Utilities for testing counter-proof functionality.
#[cfg(test)]
pub mod test_utils;
