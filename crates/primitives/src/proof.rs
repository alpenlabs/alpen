use std::fmt::{self, Display};

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use crate::{EvmEeBlockCommitment, L2BlockCommitment};

/// Represents a context for different types of proofs.
///
/// This enum categorizes proofs by their associated context, including the type of proof and its
/// range or scope. Each variant includes relevant metadata required to distinguish and track the
/// proof.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    BorshSerialize,
    BorshDeserialize,
    Serialize,
    Deserialize,
)]
pub enum ProofContext {
    /// Identifier for the EVM Execution Environment (EE) blocks used in generating the State
    /// Transition Function (STF) proof.
    EvmEeStf(EvmEeBlockCommitment, EvmEeBlockCommitment),

    /// Identifier for the Consensus Layer (CL) blocks used in generating the State Transition
    /// Function (STF) proof.
    ClStf(L2BlockCommitment, L2BlockCommitment),

    /// Identifier for a specific checkpoint being proven.
    Checkpoint(u64),
}

/// Represents the ZkVm host used for proof generation.
///
/// This enum identifies the ZkVm environment utilized to create a proof.
/// Available hosts:
/// - `SP1`: SP1 ZKVM.
/// - `Native`: Native ZKVM.
#[non_exhaustive]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    BorshSerialize,
    BorshDeserialize,
    Serialize,
    Deserialize,
)]
pub enum ProofZkVm {
    SP1,
    Native,
}

/// Represents a unique key for identifying any type of proof.
///
/// A `ProofKey` combines a `ProofContext` (which specifies the type of proof and its scope)
/// with a `ProofZkVm` (which specifies the ZKVM host used for proof generation).
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    BorshSerialize,
    BorshDeserialize,
    Serialize,
    Deserialize,
)]
pub struct ProofKey {
    /// The unique identifier for the proof type and its context.
    context: ProofContext,
    /// The ZKVM host used for proof generation.
    host: ProofZkVm,
}

impl ProofKey {
    pub fn new(context: ProofContext, host: ProofZkVm) -> Self {
        Self { context, host }
    }

    pub fn context(&self) -> &ProofContext {
        &self.context
    }

    pub fn host(&self) -> &ProofZkVm {
        &self.host
    }
}

impl Display for ProofKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ProofKey(context = {:?}, host = {:?})",
            self.context, self.host
        )
    }
}
