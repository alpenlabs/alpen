//! Snark account state types.

/// State root type.
pub type Root = [u8; 32];

/// Snark account's proof state, updated on a proof.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct ProofState {
    inner_state: Root,
    next_inbox_msg_idx: u64,
}
