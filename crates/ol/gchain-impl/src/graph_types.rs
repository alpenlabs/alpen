use strata_asm_checkpoint_types::CheckpointPayload;
use strata_checkpoint_types::EpochSummary;
use strata_gchain_types::{GLink, GLinkHeader, GLinkRef, GNodeRef};
use strata_identifiers::{Buf32, Epoch, EpochCommitment, OLBlockCommitment};
use strata_ol_chain_types_v1::{OLBlockHeaderV1, OLBlockV1};

/// A node in the OL chain graph: the state at rest after some block.
///
/// Both a block step and a checkpoint step arrive at the state after an
/// epoch's terminal block, so the block commitment is what identifies where in
/// the chain the node sits, regardless of how it was reached.
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct OLStateNode {
    epoch: Epoch,
    block: OLBlockCommitment,
    state_root: Buf32,
}

impl OLStateNode {
    pub fn new(epoch: Epoch, block: OLBlockCommitment, state_root: Buf32) -> Self {
        Self {
            epoch,
            block,
            state_root,
        }
    }

    /// The node a block's header describes the state after.
    pub fn from_header(header: &OLBlockHeaderV1) -> Self {
        Self::new(
            header.epoch(),
            header.compute_block_commitment(),
            *header.state_root(),
        )
    }

    /// The node an epoch summary describes the terminal state of.
    pub fn from_epoch_summary(summary: &EpochSummary) -> Self {
        Self::new(summary.epoch(), *summary.terminal(), *summary.final_state())
    }

    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub fn block(&self) -> &OLBlockCommitment {
        &self.block
    }

    pub fn state_root(&self) -> &Buf32 {
        &self.state_root
    }
}

// An OL state node is a few words of identifiers, which is already both small
// and `Copy`, so a separate commitment to it would just be a second copy of the
// same bytes.  It serves as its own ref until there's state in a node that's
// too big to carry around.
impl GNodeRef for OLStateNode {}

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

// Block and checkpoint versions will be disambiguated inside more specific
// nested types rather than as variants here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OLLinkHeader {
    Block(OLBlockHeaderV1),
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

/// A block step, with what the STF needs to check it against its parent.
#[derive(Clone, Debug)]
pub struct OLBlockLink {
    block: OLBlockV1,

    /// Header of the parent block, absent only for the genesis block.
    ///
    /// The STF checks header continuity against this.  It's carried on the
    /// link rather than looked up by the stage so that processing a link needs
    /// nothing beyond the link and the pre-state.
    parent_header: Option<OLBlockHeaderV1>,
}

impl OLBlockLink {
    pub fn new(block: OLBlockV1, parent_header: Option<OLBlockHeaderV1>) -> Self {
        Self {
            block,
            parent_header,
        }
    }

    pub fn block(&self) -> &OLBlockV1 {
        &self.block
    }

    pub fn header(&self) -> &OLBlockHeaderV1 {
        self.block.header()
    }

    pub fn parent_header(&self) -> Option<&OLBlockHeaderV1> {
        self.parent_header.as_ref()
    }
}

/// A checkpoint step, carrying the L1-observed payload the epoch's state is
/// reconstructed from.
#[derive(Clone, Debug)]
pub struct OLCheckpointLink {
    summary: EpochSummary,
    payload: CheckpointPayload,
}

impl OLCheckpointLink {
    pub fn new(summary: EpochSummary, payload: CheckpointPayload) -> Self {
        Self { summary, payload }
    }

    pub fn summary(&self) -> &EpochSummary {
        &self.summary
    }

    pub fn payload(&self) -> &CheckpointPayload {
        &self.payload
    }
}

#[derive(Clone, Debug)]
pub enum OLLink {
    Block(OLBlockLink),
    Checkpoint(OLCheckpointLink),
}

impl GLink for OLLink {
    fn check_structurally_consistent(&self) -> bool {
        match self {
            // Same body commitment check the OL STF makes, so that a provider
            // handing back a block whose body was altered is caught before any
            // stage processes it.  The parent header has to actually be the
            // parent, or continuity checks would be run against the wrong
            // block.
            OLLink::Block(link) => {
                let header = link.header();
                let body_matches =
                    link.block().body().compute_hash_commitment() == *header.body_root();
                let parent_matches = link
                    .parent_header()
                    .is_none_or(|ph| ph.compute_blkid() == *header.parent_blkid());
                body_matches && parent_matches
            }

            // The payload has to be for the epoch the summary describes, or
            // the reconstructed state would be checked against the wrong
            // terminal.
            OLLink::Checkpoint(link) => {
                let tip = link.payload().new_tip();
                tip.epoch == link.summary().epoch()
                    && tip.l2_commitment() == link.summary().terminal()
            }
        }
    }
}
