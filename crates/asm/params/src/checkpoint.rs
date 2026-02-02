use strata_identifiers::{L1Height, OLBlockId};
use strata_predicate::PredicateKey;

/// Checkpoint subprotocol configuration.
#[derive(Clone, Debug, PartialEq)]
pub struct CheckpointConfig {
    /// Predicate for sequencer signature verification.
    pub sequencer_predicate: PredicateKey,
    /// Predicate for checkpoint ZK proof verification.
    pub checkpoint_predicate: PredicateKey,
    /// Genesis L1 block height.
    pub genesis_l1_height: L1Height,
    /// Genesis OL block ID.
    pub genesis_ol_blkid: OLBlockId,
}
