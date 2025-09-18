use alpen_ee_primitives::{L1BlockId, OlBlockCommitment, OlBlockId};
use futures::Stream;

use crate::account_state::OlState;

pub trait OLClient {
    /// Get the best OL block
    fn tip_block(&self) -> OlBlockCommitment;
    /// Get OL state at given (OL) block `height`.
    fn state_at_height(&self, height: u64) -> Option<OlState>;
    /// Get OL state at given `ol_blockid`.
    fn state_at_blockid(&self, ol_blockid: OlBlockId) -> Option<OlState>;
    /// Get OL state corresponding to the last checkpoint posted upto given `l1_blockid`.
    fn checkpointed_state(&self, l1_blockid: L1BlockId) -> Option<OlState>;
}

pub trait OLUpdateNotifier {
    /// Subscribe to new OL blocks
    fn subscribe(&self) -> impl Stream<Item = OlState>;
}
