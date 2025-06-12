use jsonrpsee::http_client::HttpClient;
use strata_db::traits::ProofDatabase;
use strata_rocksdb::prover::db::ProofDb;
use strata_rpc_api::StrataSequencerApiClient;
use strata_rpc_types::ProofKey;
use tracing::info;

use super::errors::{CheckpointError, CheckpointResult};
use crate::errors::ProvingTaskError::{DatabaseError, ProofNotFound};

/// Submits checkpoint proof to the sequencer.
pub(crate) async fn submit_checkpoint_proof(
    checkpoint_index: u64,
    sequencer_client: &HttpClient,
    proof_key: &ProofKey,
    proof_db: &ProofDb,
) -> CheckpointResult<()> {
    let proof = proof_db
        .get_proof(proof_key)
        .map_err(DatabaseError)?
        .ok_or(ProofNotFound(*proof_key))?;

    info!(%proof_key, %checkpoint_index, "submitting ready checkpoint proof");

    sequencer_client
        .submit_checkpoint_proof(checkpoint_index, proof)
        .await
        .map_err(|e| CheckpointError::SubmitProofError {
            index: checkpoint_index,
            error: e.to_string(),
        })
}
