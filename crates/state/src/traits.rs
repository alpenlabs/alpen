use crate::chain_state::Chainstate;

/// This is any representation of data extracted from `SignedCheckpoint` that implements a method to
/// update previous chainstate data. Can be further generalized internally to use in contexts
/// other than checkpoint.
pub trait ChainstateUpdate {
    /// Apply state update to chainstate to get new chainstate.
    fn apply_to_chainstate(&self, chainstate: Option<&Chainstate>) -> Chainstate;
}

/// DA Scheme for extracting `ChainstateUpdate` from checkpoint data. Currently we are storing the
/// entire chainstate data as a part of the sidecar field in deserialized checkpoint transaction,
/// however this is subject to change in future implementation. Use a scheme that implements this
/// trait wherever we need to extract chainstate data (DA) from `SignedCheckpoint`.
pub trait ChainstateDA {
    /// Extract state update data from checkpoint
    fn chainstate_update_from_bytes(bytes: &[u8]) -> std::io::Result<impl ChainstateUpdate>;
}
