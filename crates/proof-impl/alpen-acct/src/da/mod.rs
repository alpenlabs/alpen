//! Guest-side DA correctness checks for the EE outer proof.

use ssz::Decode;
use strata_ee_acct_runtime::{ArchivedDaWitness, ArchivedEePrivateInput};
use strata_ee_chain_types::ChunkTransition;
use strata_snark_acct_types::UpdateProofPubParams;

mod blob;
mod constants;
mod error;
mod inclusion;
#[cfg(test)]
mod tests;

pub use blob::{DaBlob, EvmHeaderSummary};
pub use error::DaVerificationError;
pub use inclusion::{
    bitcoin_merkle_root, bitcoin_merkle_root_from_archived_proof, bitcoin_merkle_root_from_proof,
};

use self::{
    blob::{
        extract_da_chunks, reassemble_da_blob, verify_da_blob_metadata,
        verify_state_diff_against_chunks,
    },
    inclusion::verify_block_witness,
};

/// Runs DA correctness checks for the outer proof.
pub fn verify_da_witness(
    ee_input: &ArchivedEePrivateInput,
    da_witness: &ArchivedDaWitness,
    pub_params: &UpdateProofPubParams,
    expected_pre_state_root: [u8; 32],
) -> Result<Option<DaBlob>, DaVerificationError> {
    if da_witness.blocks().is_empty() {
        return if ee_input.chunks().is_empty() {
            Ok(None)
        } else {
            Err(DaVerificationError::MissingDaWitness)
        };
    }

    if ee_input.chunks().is_empty() {
        return Err(DaVerificationError::NoChunks);
    }

    let mut included_txs = Vec::new();
    for block in da_witness.blocks() {
        included_txs.extend(verify_block_witness(block, pub_params.ledger_refs())?);
    }

    let encoded_chunks = extract_da_chunks(included_txs.iter())?;
    let blob = reassemble_da_blob(&encoded_chunks).map_err(DaVerificationError::Reassembly)?;
    let last_chunk = decode_last_chunk_transition(ee_input)?;
    verify_da_blob_metadata(&blob, &last_chunk, pub_params, da_witness.known_bytecodes())?;

    let raw_pre_state = ee_input.raw_partial_pre_state();
    if raw_pre_state.is_empty() {
        return Err(DaVerificationError::MissingPartialPreState);
    }
    verify_state_diff_against_chunks(raw_pre_state, expected_pre_state_root, &blob, &last_chunk)?;

    Ok(Some(blob))
}

fn decode_last_chunk_transition(
    ee_input: &ArchivedEePrivateInput,
) -> Result<ChunkTransition, DaVerificationError> {
    let chunks = ee_input.chunks();
    let last = chunks.last().ok_or(DaVerificationError::NoChunks)?;
    ChunkTransition::from_ssz_bytes(last.chunk_transition_ssz())
        .map_err(DaVerificationError::LastChunkDecode)
}
