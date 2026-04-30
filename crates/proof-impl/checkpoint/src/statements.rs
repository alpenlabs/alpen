//! Orchestration Layer State Transition Function (OL STF) proof program.
//!
//! This module implements the zkVM guest program that proves correct execution
//! of the OL STF across a batch of blocks, producing a [`CheckpointClaim`] as output.

use ssz::{Decode, Encode};
use ssz_primitives::FixedBytes;
use strata_asm_manifest_types::compute_asm_manifests_hash;
use strata_asm_proto_checkpoint_types::{CheckpointClaim, L2BlockRange, TerminalHeaderComplement};
use strata_crypto::hash;
use strata_ledger_types::IStateAccessor;
use strata_ol_chain_types_new::{OLBlock, OLBlockHeader, OLLog, OLTxSegment};
use strata_ol_da::{OLDaSchemeV1, decode_ol_da_payload_bytes};
use strata_ol_state_support_types::MemoryStateBaseLayer;
use strata_ol_state_types::OLState;
use strata_ol_stf::{
    BlockComponents, BlockContext, BlockInfo, EpochInfo, construct_block,
    verify_epoch_preseal_with_diff,
};
use zkaleido::ZkVmEnv;

/// Processes a batch of OL blocks and generates a checkpoint claim.
///
/// This function is the main entry point for the OL STF proof program. It handles
/// zkVM I/O operations: reading inputs and committing outputs.
///
/// # Inputs (read from zkVM)
///
/// - Initial OL state (SSZ-encoded [`OLState`])
/// - Block batch (SSZ-encoded `Vec<OLBlock>`)
/// - Parent block header (SSZ-encoded [`OLBlockHeader`])
/// - DA state diff bytes (strata-codec encoded [`strata_ol_da::OLDaPayloadV1`])
///
/// # Outputs (committed to zkVM)
///
/// - Checkpoint claim (SSZ-encoded [`CheckpointClaim`])
///
/// # Panics
///
/// This function panics if any SSZ deserialization fails.
/// See [`process_ol_stf_core`] for additional panic conditions.
pub fn process_ol_stf(zkvm: &impl ZkVmEnv) {
    // Read and deserialize the initial OL state from zkVM input
    let initial_state_ssz_bytes = zkvm.read_buf();
    let state = OLState::from_ssz_bytes(&initial_state_ssz_bytes)
        .expect("failed to deserialize initial OL state from SSZ bytes");

    // Read and deserialize the batch of blocks to process from zkVM input
    let blocks_ssz_bytes = zkvm.read_buf();
    let blocks: Vec<OLBlock> = Vec::<OLBlock>::from_ssz_bytes(&blocks_ssz_bytes)
        .expect("failed to deserialize block batch from SSZ bytes");

    // Read and deserialize the parent block header from zkVM input
    // This header's state root must match the initial state's root
    let parent_ssz_bytes = zkvm.read_buf();
    let parent = OLBlockHeader::from_ssz_bytes(&parent_ssz_bytes)
        .expect("failed to deserialize parent block header from SSZ bytes");

    // Read DA diff witness bytes from zkVM input
    let da_state_diff_bytes = zkvm.read_buf();

    // Execute the core STF logic to get the claim
    let claim = process_ol_stf_core(state, blocks, parent, da_state_diff_bytes);

    // Serialize and commit the checkpoint claim to the zkVM as public output
    let claim_ssz_bytes = claim.as_ssz_bytes();
    zkvm.commit_buf(&claim_ssz_bytes);
}

/// Core OL STF computation logic.
///
/// This function contains the pure computation logic for processing a batch of OL blocks,
/// separated from zkVM I/O operations for testability and clarity.
///
/// It:
/// 1. Validates state consistency between parent block and initial state
/// 2. Applies each block's state transition sequentially
/// 3. Accumulates ASM manifests and OL logs across the batch
/// 4. Constructs and returns a [`CheckpointClaim`]
///
/// # Panics
///
/// This function panics if:
/// - The parent state root doesn't match the initial state root
/// - The block batch is empty
/// - Any block execution fails
/// - The computed block header doesn't match the input block header
pub fn process_ol_stf_core(
    state: OLState,
    blocks: Vec<OLBlock>,
    parent: OLBlockHeader,
    da_state_diff_bytes: Vec<u8>,
) -> CheckpointClaim {
    // Wrap OLState in MemoryStateBaseLayer to satisfy IStateAccessor requirements.
    let mut state = MemoryStateBaseLayer::new(state);

    // Verify that the parent block's state root matches the initial state's computed root.
    // This ensures state continuity and prevents invalid state transitions.
    let initial_state_root = state
        .compute_state_root()
        .expect("failed to compute initial state root");
    assert_eq!(
        *parent.state_root(),
        initial_state_root,
        "parent block state root ({:?}) does not match initial state root ({:?})",
        parent.state_root(),
        initial_state_root
    );

    // The block batch must contain at least one block to process
    assert!(
        !blocks.is_empty(),
        "block batch is empty; at least one block is required"
    );

    // The parent header must be the terminal block of the previous epoch so that
    // `prev_terminal` passed to DA verification is correct.
    assert!(
        parent.is_terminal(),
        "parent header must be the terminal block of the previous epoch"
    );

    // Capture epoch-start state for DA witness verification.
    let initial_state = MemoryStateBaseLayer::new(state.state().clone());

    // SAFETY: blocks is guaranteed non-empty by the assertion above.
    // Validate the last block is terminal before accessing its L1 update.
    let terminal_input_block = blocks
        .last()
        .expect("blocks is non-empty, verified by assertion above");
    assert!(
        terminal_input_block.header().is_terminal(),
        "last block in batch must be marked terminal in its header"
    );
    let terminal_l1_update = terminal_input_block
        .body()
        .l1_update()
        .expect("terminal checkpoint block must include an L1 update with manifests");

    // Execute all blocks in the batch and collect execution artifacts.
    // Header equality checks inside execution bind manifests/preseal commitments via body_root.
    let (logs, asm_manifests_hash, terminal_header) =
        execute_block_batch(&mut state, &blocks, &parent);

    let start = parent.compute_block_commitment();
    let end = terminal_header.compute_block_commitment();
    let l2_range = L2BlockRange::new(start, end);

    let epoch = terminal_header.epoch();
    assert_eq!(
        parent.epoch() + 1,
        epoch,
        "epoch invariant violated: expected epoch {} (parent + 1), found epoch {} in terminal block",
        parent.epoch() + 1,
        epoch
    );

    // Verify the DA witness by reconstructing epoch state from the diff.
    // Manifest processing and final state root are already proven correct by the
    // first-pass header equality checks in `execute_block_batch`, so we only
    // need to verify the preseal state root here (avoiding duplicate manifest
    // proving in the zkVM guest).
    let payload = decode_ol_da_payload_bytes(&da_state_diff_bytes)
        .expect("failed to decode OL DA payload bytes with strata_codec");
    let epoch_info = EpochInfo::new(
        BlockInfo::from_header(&terminal_header),
        parent.compute_block_commitment(),
    );
    let mut reconstructed_state = initial_state;
    verify_epoch_preseal_with_diff::<MemoryStateBaseLayer, OLDaSchemeV1>(
        &mut reconstructed_state,
        &epoch_info,
        payload,
        terminal_l1_update.preseal_state_root(),
    )
    .expect("DA witness does not match authenticated preseal state root");
    let state_diff_hash = FixedBytes::<32>::from(hash::raw(&da_state_diff_bytes));

    // Derive the terminal header subset hash from the proven terminal header.
    // This binds the sidecar's TerminalHeaderComplement to the actual executed header,
    // preventing a malicious sequencer from posting valid proofs with mismatched
    // sidecar data (the L1 verifier reconstructs this hash from sidecar fields and
    // checks it against the proof).
    let expected_terminal_header_complement = TerminalHeaderComplement::new(
        terminal_header.timestamp(),
        *terminal_header.parent_blkid(),
        *terminal_header.body_root(),
        *terminal_header.logs_root(),
    );
    let terminal_header_complement_hash = expected_terminal_header_complement.compute_hash();

    // Compute the hash of all accumulated OL logs for the checkpoint claim
    let ol_logs_hash = FixedBytes::<32>::from(hash::raw(&logs.as_ssz_bytes()));

    // Construct the checkpoint claim containing:
    // - epoch: The epoch number of the batch
    // - l2_range: The block range from parent to terminal block
    // - asm_manifests_hash: Hash of all ASM manifests in the batch
    // - state_diff_hash: Hash of witnessed DA diff bytes validated against preseal/final roots
    // - ol_logs_hash: Hash of all logs emitted during batch execution
    // - terminal_header_complement_hash: Hash binding terminal header subset from sidecar data
    CheckpointClaim::new(
        epoch,
        l2_range,
        asm_manifests_hash,
        state_diff_hash,
        ol_logs_hash,
        terminal_header_complement_hash,
    )
}

/// Executes a batch of blocks and collects execution artifacts.
///
/// Processes each block sequentially, applying state transitions to the provided state
/// and accumulating logs and ASM manifest hashes along the way.
///
/// # Arguments
///
/// * `state` - Mutable reference to the OL state to apply transitions to
/// * `blocks` - Slice of blocks to execute
/// * `initial_parent` - The parent block header for the first block in the batch
///
/// # Returns
///
/// A tuple containing:
/// - `Vec<OLLog>`: All logs emitted during block execution
/// - `FixedBytes<32>`: Hash of ASM manifests encountered in the batch
///
/// # Panics
///
/// Panics if:
/// - Any block execution fails
/// - The computed block header doesn't match the input block header
fn execute_block_batch(
    state: &mut MemoryStateBaseLayer,
    blocks: &[OLBlock],
    initial_parent: &OLBlockHeader,
) -> (Vec<OLLog>, FixedBytes<32>, OLBlockHeader) {
    // Exactly one block per epoch must carry an L1 update (the terminal block).
    // The manifest hash is computed by overwriting a single `Option` in the loop
    // below, so multiple L1 updates would silently drop earlier hashes and zero
    // would leave it as `None`.
    let l1_update_count = blocks
        .iter()
        .filter(|b| b.body().l1_update().is_some())
        .count();
    assert!(
        l1_update_count == 1,
        "proof soundness: exactly one block per epoch must carry an L1 update, found {}",
        l1_update_count
    );

    let mut parent = initial_parent.clone();
    let mut asm_manifests_hash: Option<FixedBytes<32>> = None;
    let mut logs = Vec::new();

    // Process each block in the batch sequentially, applying state transitions
    for block in blocks {
        // Extract block metadata and create execution context
        let info = BlockInfo::from_header(block.header());
        let context = BlockContext::new(&info, Some(&parent));

        // Extract the transaction segment from the block body.
        // If the block has no transactions, use an empty segment.
        let empty_tx_segment =
            OLTxSegment::new(vec![]).expect("empty transaction segment construction is infallible");
        let tx_segment = block
            .body()
            .tx_segment()
            .unwrap_or(&empty_tx_segment)
            .clone();

        // Extract L1 update (ASM manifests) if present in the block.
        // When present, compute the hash of all manifests in this update.
        let manifest_container = block
            .body()
            .l1_update()
            .map(|update| {
                asm_manifests_hash = Some(compute_asm_manifests_hash(
                    update.manifest_cont().manifests(),
                ));
                update.manifest_cont()
            })
            .cloned();

        // Assemble block components for state transition execution
        let components = BlockComponents::new(tx_segment, manifest_container);

        // Execute the block's state transition function.
        // This applies transactions, processes manifests, and updates state.
        let output = construct_block(state, context, components).expect(
            "block execution failed; all blocks in proof input must be valid and executable",
        );

        // Accumulate logs emitted during this block's execution
        logs.extend_from_slice(output.outputs().logs());

        // Verify that the computed block header matches the input block header.
        // This ensures the block was executed correctly and deterministically.
        assert_eq!(
            output.completed_block().header(),
            block.header(),
            "computed block header does not match input block header at slot {}",
            block.header().slot()
        );

        // Update parent reference for the next iteration
        parent = output.completed_block().header().clone();
    }

    // Guaranteed by the l1_update_count == 1 assertion above.
    let asm_manifests_hash =
        asm_manifests_hash.expect("exactly one L1 update per epoch is enforced above");

    (logs, asm_manifests_hash, parent)
}
