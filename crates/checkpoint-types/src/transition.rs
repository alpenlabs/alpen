use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_identifiers::{Buf32, Epoch};

/// Contains transition information in a batch checkpoint, verified by the proof
#[derive(
    Clone,
    Copy,
    Debug,
    Eq,
    PartialEq,
    Arbitrary,
    BorshDeserialize,
    BorshSerialize,
    Deserialize,
    Serialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
pub struct BatchTransition {
    /// Epoch
    pub epoch: Epoch,

    /// Transition commitment for `Chainstate`
    pub chainstate_transition: ChainstateRootTransition,
}

/// Contains Chainstate root transition information within a batch, verified by the associated
/// proof.
///
/// This struct represents a concise summary of the Chainstate transition by capturing only the
/// state roots before and after the execution of a batch of blocks. It serves as an efficient means
/// to verify state changes without storing the entire Chainstate.
///
/// # Example
///
/// Given a batch execution transitioning from block `M` to block `N`:
/// - `pre_state_root` represents the Chainstate root immediately **before** executing block `M`.
///   i.e. immediately after executing block `M-1`
/// - `post_state_root` represents the Chainstate root immediately **after** executing block `N`.
#[derive(
    Clone,
    Copy,
    Debug,
    Eq,
    PartialEq,
    Arbitrary,
    BorshDeserialize,
    BorshSerialize,
    Deserialize,
    Serialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
pub struct ChainstateRootTransition {
    /// Chainstate root prior to execution of the batch.
    ///
    /// This root reflects the state at the start of the batch transition.
    pub pre_state_root: Buf32,

    /// Chainstate root after batch execution.
    ///
    /// Represents the state of the chain immediately following the successful execution of the
    /// batch.
    pub post_state_root: Buf32,
}
