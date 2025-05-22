use crate::chain_state::Chainstate;

pub trait ChainstateUpdate {
    /// Apply state update to chainstate to get new chainstate.
    fn apply_to_chainstate(&self, chainstate: Option<&Chainstate>) -> Chainstate;
}

pub trait ChainstateDA {
    /// Extract state update data from checkpoint
    fn chainstate_update_from_bytes(bytes: &[u8]) -> std::io::Result<impl ChainstateUpdate>;
}
