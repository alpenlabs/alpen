//! Orchestration Layer State Transition Function (OL STF) proof program.
//!
//! This module implements the zkVM guest program that proves correct execution
//! of the OL STF across a batch of blocks, producing a [`CheckpointClaim`] as output.

use ssz::{Decode, Encode};
use ssz_primitives::FixedBytes;
use strata_checkpoint_types_ssz::{CheckpointClaim, L2BlockRange};
use strata_crypto::hash;
use strata_identifiers::OLBlockCommitment;
use strata_ledger_types::{AsmManifest, IStateAccessor};
use strata_ol_chain_types_new::{OLBlock, OLBlockHeader, OLLog, OLTxSegment};
use strata_ol_state_types::OLState;
use strata_ol_stf::{BlockComponents, BlockContext, BlockInfo, construct_block};
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

    // Execute the core STF logic to get the claim
    let claim = process_ol_stf_core(state, blocks, parent);

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
    mut state: OLState,
    blocks: Vec<OLBlock>,
    parent: OLBlockHeader,
) -> CheckpointClaim {
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

    // Construct the L2 block range for the checkpoint claim.
    // The range spans from the parent block to the last block in the batch.
    let start = OLBlockCommitment::new(parent.slot(), parent.compute_blkid());

    // SAFETY: blocks is guaranteed non-empty by the assertion above
    let last_block = blocks
        .last()
        .expect("blocks is non-empty, verified by assertion above");

    // All blocks in the batch belong to the same epoch (checkpoints span exactly one epoch).
    // We extract the epoch from the last block as it represents the terminal block of the epoch.
    let epoch = last_block.header().epoch();
    assert_eq!(
        parent.epoch() + 1,
        epoch,
        "epoch invariant violated: expected epoch {} (parent + 1), found epoch {} in last block",
        parent.epoch() + 1,
        epoch
    );

    let end = OLBlockCommitment::new(
        last_block.header().slot(),
        last_block.header().compute_blkid(),
    );
    let l2_range = L2BlockRange::new(start, end);

    // TODO: Implement after https://alpenlabs.atlassian.net/browse/STR-1366
    let state_diff_hash = FixedBytes::<32>::from([0u8; 32]);

    // Execute all blocks in the batch and collect execution artifacts
    let (logs, asm_manifests_hash) = execute_block_batch(&mut state, &blocks, &parent);

    // Compute the hash of all accumulated OL logs for the checkpoint claim
    let ol_logs_hash = FixedBytes::<32>::from(hash::raw(&logs.as_ssz_bytes()));

    // Construct the checkpoint claim containing:
    // - epoch: The epoch number of the batch
    // - l2_range: The block range from parent to last block
    // - asm_manifests_hash: Hash of all ASM manifests in the batch
    // - state_diff_hash: Placeholder for future state diff tracking
    // - ol_logs_hash: Hash of all logs emitted during batch execution
    CheckpointClaim::new(
        epoch,
        l2_range,
        asm_manifests_hash,
        state_diff_hash,
        ol_logs_hash,
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
    state: &mut OLState,
    blocks: &[OLBlock],
    initial_parent: &OLBlockHeader,
) -> (Vec<OLLog>, FixedBytes<32>) {
    let mut parent = initial_parent.clone();
    let mut asm_manifests_hash = FixedBytes::<32>::from([0u8; 32]);
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
                // Update the running ASM manifests hash with this block's manifests
                asm_manifests_hash = compute_asm_manifests_hash(update.manifest_cont().manifests());
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

    (logs, asm_manifests_hash)
}

/// Computes a commitment hash over a sequence of ASM manifests.
///
/// This function concatenates the individual hashes of each manifest and
/// hashes the resulting byte sequence to produce a single commitment value.
fn compute_asm_manifests_hash(manifests: &[AsmManifest]) -> FixedBytes<32> {
    // Pre-allocate buffer for concatenated manifest hashes
    // Each manifest hash is 32 bytes
    let mut manifest_hashes_buf = Vec::with_capacity(manifests.len() * 32);

    // Concatenate individual manifest hashes
    for manifest in manifests {
        let manifest_hash = manifest.compute_hash();
        manifest_hashes_buf.extend_from_slice(&manifest_hash);
    }

    // Compute final commitment hash over the concatenated hashes
    hash::raw(&manifest_hashes_buf).into()
}
