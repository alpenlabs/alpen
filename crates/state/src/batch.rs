use alpen_express_primitives::buf::{Buf32, Buf64};
use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};

use crate::{id::L2BlockId, l1::L1BlockId};

/// Public parameters for batch proof to be posted to DA.
/// Will be updated as prover specs evolve.
#[derive(Clone, Debug, PartialEq, BorshDeserialize, BorshSerialize, Arbitrary)]
pub struct BatchCommitment {
    /// Safe L1 block for the batch
    l1blockid: L1BlockId,
    /// Last L2 block covered by the batch
    l2blockid: L2BlockId,
}

impl BatchCommitment {
    pub fn new(l1blockid: L1BlockId, l2blockid: L2BlockId) -> Self {
        Self {
            l1blockid,
            l2blockid,
        }
    }

    pub fn l1blockid(&self) -> &L1BlockId {
        &self.l1blockid
    }

    pub fn l2blockid(&self) -> &L2BlockId {
        &self.l2blockid
    }

    pub fn get_sighash(&self) -> Buf32 {
        let mut buf = [0; 32 + 32];

        buf[0..32].copy_from_slice(self.l1blockid.as_ref());
        buf[32..].copy_from_slice(self.l2blockid.as_ref());

        alpen_express_primitives::hash::raw(&buf)
    }
}

#[derive(Clone, Debug, BorshDeserialize, BorshSerialize)]
pub struct SignedBatchCommitment {
    inner: BatchCommitment,
    signature: Buf64,
}

impl SignedBatchCommitment {
    pub fn new(inner: BatchCommitment, signature: Buf64) -> Self {
        Self { inner, signature }
    }
}

impl From<SignedBatchCommitment> for BatchCommitment {
    fn from(value: SignedBatchCommitment) -> Self {
        value.inner
    }
}
