use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_primitives::prelude::*;

use crate::{
    exec_update,
    header::{L2BlockHeader, SignedL2BlockHeader},
    id::L2BlockId,
};

/// Full contents of the bare L2 block.
#[derive(Clone, Debug, Eq, PartialEq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct L2Block {
    /// Header that links the block into the L2 block chain and carries the
    /// block's credential from a sequencer.
    pub(crate) header: SignedL2BlockHeader,

    /// Body that contains the bulk of the data.
    pub(crate) body: L2BlockBody,
}

impl L2Block {
    pub fn new(header: SignedL2BlockHeader, body: L2BlockBody) -> Self {
        Self { header, body }
    }

    pub const fn header(&self) -> &SignedL2BlockHeader {
        &self.header
    }

    pub const fn body(&self) -> &L2BlockBody {
        &self.body
    }

    pub const fn l1_segment(&self) -> &L1Segment {
        &self.body.l1_segment
    }

    pub const fn exec_segment(&self) -> &ExecSegment {
        &self.body.exec_segment
    }

    pub fn into_parts(self) -> (SignedL2BlockHeader, L2BlockBody) {
        let Self { header, body } = self;

        (header, body)
    }
}

/// Careful impl that makes the header consistent with the body.  But the prev
/// block is always 0 and the state root is random.
impl<'a> Arbitrary<'a> for L2Block {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let body = L2BlockBody::arbitrary(u)?;
        let slot = u64::arbitrary(u)?;
        let epoch = u64::arbitrary(u)?;
        let ts = u64::arbitrary(u)?;
        let prev = L2BlockId::from(Buf32::zero());
        let sr = Buf32::arbitrary(u)?;
        let header = L2BlockHeader::new(slot, epoch, ts, prev, &body, sr);
        let signed_header = SignedL2BlockHeader::new(header, Buf64::arbitrary(u)?);
        Ok(Self::new(signed_header, body))
    }
}

/// Contains the additional payloads within the L2 block.
#[derive(
    Clone, Debug, Eq, PartialEq, Arbitrary, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
pub struct L2BlockBody {
    l1_segment: L1Segment,
    exec_segment: ExecSegment,
}

impl L2BlockBody {
    pub fn new(l1_segment: L1Segment, exec_segment: ExecSegment) -> Self {
        Self {
            l1_segment,
            exec_segment,
        }
    }

    pub const fn l1_segment(&self) -> &L1Segment {
        &self.l1_segment
    }

    pub const fn exec_segment(&self) -> &ExecSegment {
        &self.exec_segment
    }
}

/// Container for additional messages that we've observed from the L1, if there
/// are any.
///
/// Soon this will be refactored so that it only includes the new tip's
/// `L1BlockCommitment` and we will "sideload" the blocks in the epoch
/// finalization.  So the manifests here contains kinda a lot of data, but it
/// won't be present in DA payloads so it's fine.
#[derive(
    Clone, Debug, Eq, PartialEq, Arbitrary, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
pub struct L1Segment {
    /// New L1 block height.  This should correspond with the last manifest in
    /// the new_manifests, or the current chainstate height if it's not being
    /// extended.
    ///
    /// This partly serves as a safety measure to make sure we don't update the
    /// block heights wrong.
    new_height: u64,

    /// New manifests that we've seen from L1 that we didn't see in the previous
    /// L2 block.
    new_manifests: Vec<L1BlockManifest>,
}

impl L1Segment {
    /// Constructs a new instance.  These new manifests MUST be sorted in order
    /// of block height.
    pub fn new(new_height: u64, new_manifests: Vec<L1BlockManifest>) -> Self {
        Self {
            new_height,
            new_manifests,
        }
    }

    pub fn new_empty(cur_height: u64) -> Self {
        Self::new(cur_height, Vec::new())
    }

    pub const fn new_height(&self) -> u64 {
        self.new_height
    }

    pub fn new_manifests(&self) -> &[L1BlockManifest] {
        &self.new_manifests
    }

    /// Returns a the new tip L1 blkid, if there is one and this is
    /// well-formed.
    pub fn new_tip_blkid(&self) -> Option<L1BlockId> {
        self.new_manifests().last().map(|mf| *mf.blkid())
    }
}

/// Information relating to how to update the execution layer.
///
/// Right now this just contains a single execution update since we only have a
/// single execution environment in our execution layer.
#[derive(
    Clone, Debug, Eq, PartialEq, Arbitrary, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
pub struct ExecSegment {
    /// Update payload for the single execution environment.
    update: exec_update::ExecUpdate,
}

impl ExecSegment {
    pub fn new(update: exec_update::ExecUpdate) -> Self {
        Self { update }
    }

    /// The EE update payload.
    ///
    /// This might be replaced with a totally different scheme if we have
    /// multiple EEs.
    pub const fn update(&self) -> &exec_update::ExecUpdate {
        &self.update
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Arbitrary, BorshSerialize, BorshDeserialize)]
pub struct L2BlockAccessory {
    exec_payload: Vec<u8>,
    gas_used: u64,
}

impl L2BlockAccessory {
    pub fn new(exec_payload: Vec<u8>, gas_used: u64) -> Self {
        Self {
            exec_payload,
            gas_used,
        }
    }

    pub fn exec_payload(&self) -> &[u8] {
        &self.exec_payload
    }

    pub const fn gas_used(&self) -> u64 {
        self.gas_used
    }
}

#[derive(Clone, Debug, Eq, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary)]
pub struct L2BlockBundle {
    block: L2Block,
    accessory: L2BlockAccessory,
}

impl L2BlockBundle {
    pub fn new(block: L2Block, accessory: L2BlockAccessory) -> Self {
        Self { block, accessory }
    }

    pub const fn block(&self) -> &L2Block {
        &self.block
    }

    pub const fn accessory(&self) -> &L2BlockAccessory {
        &self.accessory
    }

    pub const fn header(&self) -> &SignedL2BlockHeader {
        self.block.header()
    }

    pub const fn body(&self) -> &L2BlockBody {
        self.block.body()
    }

    pub const fn l1_segment(&self) -> &L1Segment {
        self.block.l1_segment()
    }

    pub const fn exec_segment(&self) -> &ExecSegment {
        self.block.exec_segment()
    }

    pub fn into_parts(self) -> (L2Block, L2BlockAccessory) {
        let Self { block, accessory } = self;

        (block, accessory)
    }
}

impl From<L2BlockBundle> for L2Block {
    fn from(value: L2BlockBundle) -> Self {
        value.block
    }
}

#[cfg(test)]
mod tests {
    use strata_test_utils::ArbitraryGenerator;

    use super::*;
    use crate::block_validation::validate_block_segments;

    // This test is flaky because sometimes it generates an L1 segment with no
    // elements twice.
    #[test]
    fn test_verify_block_hashes() {
        // use arbitrary generator to get the new block
        let block: L2Block = ArbitraryGenerator::new().generate();
        assert!(validate_block_segments(&block));

        let arb_exec_segment: ExecSegment = ArbitraryGenerator::new().generate();
        let arb_l1_segment: L1Segment = ArbitraryGenerator::new().generate();
        // mutate the l2Block's body to create a new block with arbitrary exec segment
        let blk_body = L2BlockBody::new(block.body().l1_segment().clone(), arb_exec_segment);
        let arb_exec_block = L2Block::new(block.header().clone(), blk_body);
        assert!(!validate_block_segments(&arb_exec_block));

        // mutate the l2Block's body to create a new block with arbitrary l1 segment
        let blk_body = L2BlockBody::new(arb_l1_segment, block.body().exec_segment().clone());
        let arb_l1_block = L2Block::new(block.header().clone(), blk_body);
        assert!(!validate_block_segments(&arb_l1_block));
    }
}
