use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_identifiers::{Buf32, L1BlockCommitment, L1BlockId};
use strata_btc_types::{L1HeaderRecord, L1Tx};

/// Reference to a Bitcoin transaction by block ID and transaction index.
#[derive(
    Clone, Debug, PartialEq, Eq, Arbitrary, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
pub struct L1TxRef(L1BlockId, u32);

impl L1TxRef {
    pub fn new(blkid: L1BlockId, idx: u32) -> Self {
        Self(blkid, idx)
    }

    pub fn blkid(&self) -> &L1BlockId {
        &self.0
    }

    pub fn idx(&self) -> u32 {
        self.1
    }
}

impl From<L1TxRef> for (L1BlockId, u32) {
    fn from(val: L1TxRef) -> Self {
        (val.0, val.1)
    }
}

impl From<(L1BlockId, u32)> for L1TxRef {
    fn from(val: (L1BlockId, u32)) -> Self {
        Self::new(val.0, val.1)
    }
}

impl From<(&L1BlockId, u32)> for L1TxRef {
    fn from(val: (&L1BlockId, u32)) -> Self {
        Self::new(*val.0, val.1)
    }
}
