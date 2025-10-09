use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use crate::{L1TxProof, RawBitcoinTx};
use super::protocol_operation::ProtocolOperation;

/// Bitcoin-anchored transaction with proof and protocol operations.
#[derive(
    Clone, Debug, BorshSerialize, BorshDeserialize, PartialEq, Eq, Arbitrary, Serialize, Deserialize,
)]
pub struct L1Tx {
    // TODO: verify if we need L1TxProof or L1WtxProof
    proof: L1TxProof,
    tx: RawBitcoinTx,
    protocol_ops: Vec<ProtocolOperation>,
}

impl L1Tx {
    pub fn new(proof: L1TxProof, tx: RawBitcoinTx, protocol_ops: Vec<ProtocolOperation>) -> Self {
        Self {
            proof,
            tx,
            protocol_ops,
        }
    }

    pub fn proof(&self) -> &L1TxProof {
        &self.proof
    }

    pub fn tx_data(&self) -> &RawBitcoinTx {
        &self.tx
    }

    pub fn protocol_ops(&self) -> &[ProtocolOperation] {
        &self.protocol_ops
    }
}
