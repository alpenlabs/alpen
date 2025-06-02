use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use super::{inclusion_proof::L1TxProof, ops::ProtocolOperation, RawBitcoinTx};

/// Tx body with a proof.
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

    pub const fn proof(&self) -> &L1TxProof {
        &self.proof
    }

    pub const fn tx_data(&self) -> &RawBitcoinTx {
        &self.tx
    }

    #[allow(clippy::missing_const_for_fn)]
    pub fn protocol_ops(&self) -> &[ProtocolOperation] {
        &self.protocol_ops
    }
}

#[derive(
    Clone, Debug, Arbitrary, BorshDeserialize, BorshSerialize, PartialEq, Eq, Serialize, Deserialize,
)]
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

    pub const fn tx(&self) -> &L1Tx {
        &self.tx
    }

    pub const fn deposit_idx(&self) -> u32 {
        self.deposit_idx
    }
}

#[derive(
    Clone, Debug, Arbitrary, BorshDeserialize, BorshSerialize, PartialEq, Eq, Serialize, Deserialize,
)]
pub struct DaTx {
    // TODO other fields that we need to be able to identify the DA
    /// The transaction in the block.
    tx: L1Tx,
}

impl DaTx {
    pub fn new(tx: L1Tx) -> Self {
        Self { tx }
    }

    pub const fn tx(&self) -> &L1Tx {
        &self.tx
    }
}
