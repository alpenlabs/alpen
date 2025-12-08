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

/// Represents the transition of `TxFilterConfig` within an epoch, verified through an associated
/// proof.
///
/// # Overview
///
/// The `TxFilterConfigTransition` tracks the transition of the `TxFilterConfig` ensuring
/// consistency in transaction filtering parameters over time. The transition is identified by its
/// hash of the initial state `pre_config_hash` and `post_config_hash` hash of the update state.
/// Hash is computed on the borsh serialized TxFilterConfig
///
/// # Lifecycle and Dependencies
///
/// - **Initial Derivation**: `TxFilterConfig` is derived from `RollupParams`.
/// - **Epoch Updates**: At the end of each epoch, the `Chainstate` is used to update the
///   `TxFilterConfig`.
/// - **Checkpoint Inclusion**: An epoch only progresses after a valid checkpoint is posted in
///   Bitcoin. The terminal L1 block of epoch N contains the checkpoint for epoch N-1, which
///   includes the `Chainstate` at the end of epoch N-1.
/// - **Usage of Chainstate**:
///   - The new `TxFilterConfig` (based on the updated `Chainstate`) is only usable in blocks
///     following the terminal block that contains the checkpoint.
///   - For the terminal block itself and any blocks before it, the `TxFilterConfig` derived from
///     the chainstate at the end of epoch N-2 must be used.
///   - Consequently, `pre_config_hash` corresponds to the `TxFilterConfig` at the beginning of
///     epoch N-2 (or equivalently, the end of epoch N-2), and `post_config_hash` corresponds to the
///     `TxFilterConfig` at the end of epoch N-1.
///
/// # Notes
///
/// - By capturing both the old and new configuration states, this struct allows verifiers to
///   confirm that the `TxFilterConfig` evolved correctly according to the checkpointed chainstate.
/// - The transition can be checked against the proof to ensure no unauthorized modifications
///   occurred.
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
)]
pub struct TxFilterConfigTransition {
    /// Hash of the `TxFilterConfig` before the transition (derived from the chainstate at the
    /// start of epoch N-1).
    pub pre_config_hash: Buf32,
    /// Hash of the `TxFilterConfig` after the transition (derived from the chainstate at the
    /// end of epoch N-1).
    pub post_config_hash: Buf32,
}
