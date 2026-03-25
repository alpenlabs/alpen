//! EE proof task types for PaaS integration.

use serde::{Deserialize, Serialize};

use crate::BatchId;

/// Proof task variants for the EE proof pipeline.
///
/// The EE uses a two-stage proof pipeline:
/// 1. **Chunk proofs**: Prove state transitions for a chunk of execution blocks.
/// 2. **Acct proof**: Aggregates chunk proofs into a single account update proof.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum EeProofTask {
    /// Prove a chunk of blocks within a batch.
    Chunk {
        batch_id: BatchId,
        chunk_idx: u32,
    },
    /// Prove the account update aggregating all chunk proofs for a batch.
    Acct {
        batch_id: BatchId,
    },
}

/// Routing key for EE proof handler dispatch.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum EeProofVariant {
    Chunk,
    Acct,
}

#[cfg(feature = "paas")]
impl strata_paas::ProgramType for EeProofTask {
    type RoutingKey = EeProofVariant;

    fn routing_key(&self) -> EeProofVariant {
        match self {
            Self::Chunk { .. } => EeProofVariant::Chunk,
            Self::Acct { .. } => EeProofVariant::Acct,
        }
    }
}
