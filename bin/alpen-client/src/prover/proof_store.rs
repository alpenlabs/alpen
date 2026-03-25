//! In-memory proof store for EE batch proofs.
//!
//! Tracks per-batch orchestration state (chunk->acct pipeline) and stores
//! completed proof bytes indexed by [`ProofId`].

use std::{
    collections::HashMap,
    sync::RwLock,
};

use alpen_ee_common::{BatchId, Proof, ProofId};
use strata_primitives::buf::Buf32;

/// Per-batch orchestration state.
#[derive(Debug, Clone)]
pub(crate) enum BatchProofState {
    /// Chunk proofs are being generated.
    ChunksInProgress {
        chunk_uuids: Vec<String>,
        total_chunks: u32,
    },
    /// All chunks done, account proof is being generated.
    AcctInProgress {
        acct_uuid: String,
    },
    /// Proof generation completed.
    Completed {
        proof_id: ProofId,
    },
    /// Permanent failure.
    Failed {
        reason: String,
    },
}

/// Stores orchestration state per batch and completed proofs by ID.
pub(crate) struct ProofStore {
    batch_states: RwLock<HashMap<BatchId, BatchProofState>>,
    /// Completed proof bytes keyed by ProofId.
    proofs: RwLock<HashMap<ProofId, Vec<u8>>>,
}

impl ProofStore {
    pub(crate) fn new() -> Self {
        Self {
            batch_states: RwLock::new(HashMap::new()),
            proofs: RwLock::new(HashMap::new()),
        }
    }

    /// Get the current orchestration state for a batch.
    pub(crate) fn get_batch_state(&self, batch_id: &BatchId) -> Option<BatchProofState> {
        let map = self.batch_states.read().expect("lock poisoned");
        map.get(batch_id).cloned()
    }

    /// Set the orchestration state for a batch.
    pub(crate) fn set_batch_state(&self, batch_id: BatchId, state: BatchProofState) {
        let mut map = self.batch_states.write().expect("lock poisoned");
        map.insert(batch_id, state);
    }

    /// Store completed proof bytes indexed by ID.
    pub(crate) fn store_proof(&self, proof_id: ProofId, proof_bytes: Vec<u8>) {
        let mut map = self.proofs.write().expect("lock poisoned");
        map.insert(proof_id, proof_bytes);
    }

    /// Get a proof by its ID, wrapped in [`Proof`].
    pub(crate) fn get_proof(&self, proof_id: &ProofId) -> Option<Proof> {
        let map = self.proofs.read().expect("lock poisoned");
        map.get(proof_id).map(|bytes| Proof::from_vec(bytes.clone()))
    }

    /// Derive a deterministic [`ProofId`] from proof bytes.
    pub(crate) fn proof_id_from_bytes(bytes: &[u8]) -> ProofId {
        // Simple hash: take first 32 bytes or zero-pad.
        let mut buf = [0u8; 32];
        let len = bytes.len().min(32);
        buf[..len].copy_from_slice(&bytes[..len]);
        ProofId::from(Buf32(buf))
    }
}
