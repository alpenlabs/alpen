//! Outer proof (batch proof) guest logic.
//!
//! Verifies inner chunk proofs, checks state continuity, and aggregates results.

use ssz::{Decode, Encode};
use strata_codec::{decode_buf_exact, encode_to_vec};
use strata_ee_acct_types::UpdateExtraData;
use strata_identifiers::Hash;
use strata_snark_acct_types::{
    LedgerRefs, MessageEntry, ProofState, UpdateOutputs, UpdateProofPubParams,
};
use zkaleido::ZkVmEnv;

use crate::types::ChunkProofOutput;

/// Processes a batch proof in the zkVM guest.
///
/// This function:
/// 1. Reads and verifies each chunk proof using the chunk_vkey
/// 2. Verifies state continuity between chunks
/// 3. Aggregates outputs from all chunks
/// 4. Builds and commits UpdateProofPubParams
pub fn process_batch_proof(zkvm: &impl ZkVmEnv, chunk_vkey: &[u32; 8]) {
    // 1. Read number of chunk proofs
    let num_chunks: u32 = zkvm.read_serde();
    assert!(num_chunks > 0, "No chunk proofs provided");

    // 2. Read and verify each chunk proof, obtaining ChunkProofOutputs
    let chunk_outputs: Vec<ChunkProofOutput> = (0..num_chunks)
        .map(|_| {
            let verified_bytes = zkvm.read_verified_buf(chunk_vkey);
            ChunkProofOutput::from_ssz_bytes(&verified_bytes)
                .expect("Failed to decode ChunkProofOutput")
        })
        .collect();

    // 3. Verify state continuity between chunks
    verify_state_continuity(&chunk_outputs);

    // 4. TODO: DA verification (TBD)
    let ledger_refs = LedgerRefs::new_empty();

    // 5. Aggregate outputs from all chunks
    let (prev_state, new_state, message_inputs, outputs, extra_data) =
        aggregate_chunk_outputs(chunk_outputs);

    // 6. Build and commit UpdateProofPubParams
    let update_proof_params = UpdateProofPubParams::new(
        prev_state,
        new_state,
        message_inputs,
        ledger_refs,
        outputs,
        extra_data,
    );

    zkvm.commit_buf(&update_proof_params.as_ssz_bytes());
}

/// Verifies that chunk states are continuous: chunk[i].new_state == chunk[i+1].prev_state
fn verify_state_continuity(chunk_outputs: &[ChunkProofOutput]) {
    for i in 0..chunk_outputs.len() - 1 {
        assert_eq!(
            chunk_outputs[i].new_state,
            chunk_outputs[i + 1].prev_state,
            "State discontinuity between chunk {} and {}",
            i,
            i + 1
        );
    }
}

/// Aggregates messages, outputs, and extra_data from all chunks.
///
/// Returns (prev_state, new_state, messages, outputs, extra_data_bytes) for building
/// UpdateProofPubParams.
fn aggregate_chunk_outputs(
    chunk_outputs: Vec<ChunkProofOutput>,
) -> (
    ProofState,
    ProofState,
    Vec<MessageEntry>,
    UpdateOutputs,
    Vec<u8>,
) {
    let mut prev_state = None;
    let mut new_state = None;
    let mut all_messages = Vec::new();
    let mut aggregated_outputs = UpdateOutputs::new_empty();
    let mut total_processed_inputs = 0u32;
    let mut total_processed_fincls = 0u32;
    let mut final_tip_blkid = Hash::default();

    for chunk_output in chunk_outputs {
        let ChunkProofOutput {
            prev_state: chunk_prev_state,
            new_state: chunk_new_state,
            processed_messages,
            outputs,
            extra_data,
        } = chunk_output;

        // Capture first prev_state, update new_state each iteration
        if prev_state.is_none() {
            prev_state = Some(chunk_prev_state);
        }
        new_state = Some(chunk_new_state);

        // Collect messages
        all_messages.extend(processed_messages);

        // Merge outputs (zero-copy by moving fields)
        aggregated_outputs
            .try_extend_transfers(outputs.transfers)
            .expect("Failed to aggregate transfers");
        aggregated_outputs
            .try_extend_messages(outputs.messages)
            .expect("Failed to aggregate messages");

        // Decode and aggregate extra_data
        let extra: UpdateExtraData =
            decode_buf_exact(&extra_data).expect("Failed to decode UpdateExtraData");
        total_processed_inputs += *extra.processed_inputs();
        total_processed_fincls += *extra.processed_fincls();
        final_tip_blkid = *extra.new_tip_blkid();
    }

    // Build aggregated extra_data
    let aggregated_extra = UpdateExtraData::new(
        final_tip_blkid,
        total_processed_inputs,
        total_processed_fincls,
    );

    (
        prev_state.expect("At least one chunk"),
        new_state.expect("At least one chunk"),
        all_messages,
        aggregated_outputs,
        encode_to_vec(&aggregated_extra).expect("Failed to encode aggregated extra_data"),
    )
}
