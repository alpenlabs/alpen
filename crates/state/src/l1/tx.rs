use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use strata_primitives::l1::L1TxProof;

use crate::tx::ProtocolOperation;

/// Tx body with a proof.
#[derive(Clone, Debug, BorshSerialize, BorshDeserialize, PartialEq, Eq, Arbitrary)]
pub struct L1Tx {
    proof: L1TxProof,
    tx: Vec<u8>,
    protocol_operation: ProtocolOperation,
}

impl L1Tx {
    pub fn new(proof: L1TxProof, tx: Vec<u8>, protocol_operation: ProtocolOperation) -> Self {
        Self {
            proof,
            tx,
            protocol_operation,
        }
    }

    pub fn proof(&self) -> &L1TxProof {
        &self.proof
    }

    pub fn tx_data(&self) -> &[u8] {
        &self.tx
    }

    pub fn protocol_operation(&self) -> &ProtocolOperation {
        &self.protocol_operation
    }
}

#[derive(Clone, Debug, Arbitrary, BorshDeserialize, BorshSerialize, PartialEq, Eq)]
pub struct DepositUpdateTx {
    /// The transaction in the block.
    tx: L1Tx,

    /// The deposit ID that this corresponds to, so that we can update it when
    /// we mature the L1 block.  A ref to this [`L1Tx`] exists in `pending_update_txs`
    /// in the `DepositEntry` structure in state.
    deposit_idx: u32,
}

impl DepositUpdateTx {
    pub fn new(tx: L1Tx, deposit_idx: u32) -> Self {
        Self { tx, deposit_idx }
    }

    pub fn tx(&self) -> &L1Tx {
        &self.tx
    }

    pub fn deposit_idx(&self) -> u32 {
        self.deposit_idx
    }
}

#[derive(Clone, Debug, Arbitrary, BorshDeserialize, BorshSerialize, PartialEq, Eq)]
pub struct DaTx {
    // TODO other fields that we need to be able to identify the DA
    /// The transaction in the block.
    tx: L1Tx,
}

impl DaTx {
    pub fn new(tx: L1Tx) -> Self {
        Self { tx }
    }

    pub fn tx(&self) -> &L1Tx {
        &self.tx
    }
}
