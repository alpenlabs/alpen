use crate::chain_state::Chainstate;

/// This is any representation of data extracted from `SignedCheckpoint` that implements a method to
/// update previous chainstate data. Can be further generalized internally to use in contexts
/// other than checkpoint.
pub trait ChainstateDiff {
    /// Apply state update to chainstate to get new chainstate.
    fn apply_to_chainstate(&self, chainstate: &mut Chainstate) -> anyhow::Result<()>;

    /// Extract diff structure from buffer.
    fn from_buf(buf: &[u8]) -> std::io::Result<Self>
    where
        Self: Sized;
}
