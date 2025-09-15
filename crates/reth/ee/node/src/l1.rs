use alpen_ee_primitives::L1BlockCommitment;
use futures::Stream;

/// Provide a view of L1 to determine finality.
/// This can be provided through the OL's rpcs if needed.
pub trait L1Client {
    /// Get the best L1 block
    fn tip_block(&self) -> L1BlockCommitment;
    /// Get L1 block at a specified height
    fn block_at_height(&self, height: u64) -> Option<L1BlockCommitment>;
}

pub trait L1UpdateNotifier {
    /// Subscribe to new L1 blocks
    fn subscribe(&self) -> impl Stream<Item = L1BlockCommitment>;
}
