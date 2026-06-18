//! Aggregate database backend accessor trait.

use std::sync::Arc;

use crate::{
    asm::AsmDatabase, checkpoint_proof::CheckpointProofDatabase, chunked_envelope::L1ChunkedEnvelopeDatabase,
    client_state::ClientStateDatabase, l1::L1Database, l1_broadcast::L1BroadcastDatabase,
    l1_writer::L1WriterDatabase, mempool::MempoolDatabase, ol_block::OLBlockDatabase,
    ol_checkpoint::OLCheckpointDatabase, ol_state::OLStateDatabase,
    ol_state_index::OLStateIndexingDatabase, prover_task::ProverTaskDatabase,
};

/// Common database backend interface that we can parameterize worker tasks over if
/// parameterizing them over each individual trait gets cumbersome or if we need
/// to use behavior that crosses different interfaces.
pub trait DatabaseBackend: Send + Sync {
    fn asm_db(&self) -> Arc<impl AsmDatabase>;
    fn l1_db(&self) -> Arc<impl L1Database>;
    fn client_state_db(&self) -> Arc<impl ClientStateDatabase>;
    fn ol_block_db(&self) -> Arc<impl OLBlockDatabase>;
    fn ol_state_db(&self) -> Arc<impl OLStateDatabase>;
    fn ol_checkpoint_db(&self) -> Arc<impl OLCheckpointDatabase>;
    fn writer_db(&self) -> Arc<impl L1WriterDatabase>;
    fn checkpoint_proof_db(&self) -> Arc<impl CheckpointProofDatabase>;
    fn prover_task_db(&self) -> Arc<impl ProverTaskDatabase>;
    fn broadcast_db(&self) -> Arc<impl L1BroadcastDatabase>;
    fn chunked_envelope_db(&self) -> Arc<impl L1ChunkedEnvelopeDatabase>;
    fn mempool_db(&self) -> Arc<impl MempoolDatabase>;
    fn ol_state_indexing_db(&self) -> Arc<impl OLStateIndexingDatabase>;
}
