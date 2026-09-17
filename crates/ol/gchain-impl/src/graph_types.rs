use strata_checkpoint_types::EpochSummary;
use strata_gchain_types::{GLink, GLinkHeader, GLinkRef, GNode, GNodeRef};
use strata_identifiers::{Buf32, Epoch, EpochCommitment, OLBlockCommitment};
use strata_ol_chain_types_v1::{OLBlockHeaderV1, OLBlockV1};

#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct OLStateNode {
    epoch: Epoch,
    state_root: Buf32,
}

impl OLStateNode {
    pub fn new(epoch: Epoch, state_root: Buf32) -> Self {
        Self { epoch, state_root }
    }

    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub fn state_root(&self) -> &Buf32 {
        &self.state_root
    }
}

// An OL state node is an epoch number and a state root, which is already both
// small and `Copy`, so a separate commitment to it would just be a second copy
// of the same bytes.  It serves as its own ref until there's state in a node
// that's too big to carry around.
impl GNodeRef for OLStateNode {}
impl GNode for OLStateNode {}

#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum OLLinkRef {
    Block(BlockLinkRefInfo),
    Checkpoint(CkptLinkRefInfo),
}

impl From<OLBlockCommitment> for OLLinkRef {
    fn from(block: OLBlockCommitment) -> Self {
        Self::Block(BlockLinkRefInfo { block })
    }
}

impl From<EpochCommitment> for OLLinkRef {
    fn from(epoch: EpochCommitment) -> Self {
        Self::Checkpoint(CkptLinkRefInfo { epoch })
    }
}

impl GLinkRef for OLLinkRef {}

#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct BlockLinkRefInfo {
    block: OLBlockCommitment,
}

impl BlockLinkRefInfo {
    pub fn block(&self) -> &OLBlockCommitment {
        &self.block
    }
}

#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct CkptLinkRefInfo {
    epoch: EpochCommitment,
}

impl CkptLinkRefInfo {
    pub fn epoch(&self) -> &EpochCommitment {
        &self.epoch
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OLLinkHeader {
    BlockV1(OLBlockHeaderV1),
    Checkpoint(CheckpointData),
}

impl GLinkHeader for OLLinkHeader {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckpointData {
    /// Summary of the epoch the checkpoint attests to.
    ///
    /// This describes both ends of the link, since it commits to the terminal
    /// blocks of this epoch and the previous one.
    summary: EpochSummary,
    // TODO(trey): figure out what other structure to put here, why is this spread out so awkwardly?
}

impl CheckpointData {
    pub fn new(summary: EpochSummary) -> Self {
        Self { summary }
    }

    pub fn summary(&self) -> &EpochSummary {
        &self.summary
    }

    /// Commitment to the epoch the checkpoint attests to.
    pub fn get_epoch_commitment(&self) -> EpochCommitment {
        self.summary.get_epoch_commitment()
    }

    /// Commitment to the epoch before the one the checkpoint attests to, if
    /// this isn't the genesis epoch.
    pub fn get_prev_epoch_commitment(&self) -> Option<EpochCommitment> {
        self.summary.get_prev_epoch_commitment()
    }
}

#[derive(Clone, Debug)]
pub enum OLLink {
    BlockV1(OLBlockV1),
    Checkpoint(CheckpointData),
}

impl GLink for OLLink {
    fn check_structurally_consistent(&self) -> bool {
        match self {
            // Same body commitment check the OL STF makes, so that a provider
            // handing back a block whose body was altered is caught before any
            // stage processes it.
            OLLink::BlockV1(block) => {
                block.body().compute_hash_commitment() == *block.header().body_root()
            }

            // A checkpoint link carries only its summary, which has no separate
            // body to commit to.
            OLLink::Checkpoint(_) => true,
        }
    }
}
