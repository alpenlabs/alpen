use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_btc_types::{L1Tx, L1TxProof, ProtocolOperation};

/// Bitcoin-anchored deposit update transaction.
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

    pub fn tx(&self) -> &L1Tx {
        &self.tx
    }

    pub fn deposit_idx(&self) -> u32 {
        self.deposit_idx
    }
}

/// Bitcoin-anchored data availability transaction.
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

    pub fn tx(&self) -> &L1Tx {
        &self.tx
    }
}
