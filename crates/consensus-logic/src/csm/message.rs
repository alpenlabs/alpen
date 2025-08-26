use strata_state::id::L2BlockId;

/// Message about a new block the fork choice manager might do something with.
#[derive(Clone, Debug)]
pub enum ForkChoiceMessage {
    /// New block coming in from over the network to be considered.
    NewBlock(L2BlockId),
}
