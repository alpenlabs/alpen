//! High-level verification and processing flows.
//!
//! These procedures aren't useful for initial execution like during block
//! assembly as they perform all of the block validation checks including
//! outputs (corresponding with headers/checkpoints/etc).

use strata_identifiers::{Buf32, hash};
use strata_ledger_types::StateAccessor;
use strata_merkle::{BinaryMerkleTree, Sha256Hasher};
use strata_ol_chain_types_new::{
    OLBlock, OLBlockBody, OLBlockHeader, OLL1ManifestContainer, OLLog, OLTxSegment,
};
use strata_ol_da::DaScheme;

use crate::{
    chain_processing,
    context::{
        BasicExecContext, BlockContext, BlockInfo, EpochInfo, EpochInitialContext, TxExecContext,
    },
    errors::{ExecError, ExecResult},
    manifest_processing,
    output::ExecOutputBuffer,
    transaction_processing,
};

/// Commitments that we are checking against a block.
///
/// This is ultimately derived from the header (and maybe L1 update data).
#[derive(Copy, Clone, Debug)]
pub struct BlockExecExpectations {
    post_state_roots: BlockPostStateCommitments,
    logs_root: Buf32,
}

impl BlockExecExpectations {
    pub(crate) fn new(post_state_roots: BlockPostStateCommitments, logs_root: Buf32) -> Self {
        Self {
            post_state_roots,
            logs_root,
        }
    }
}

/// Describes the state roots we might compute in the different phases.
#[derive(Copy, Clone, Debug)]
pub enum BlockPostStateCommitments {
    /// For regular, non-terminal blocks.  This is just the header state root.
    Common(Buf32),

    /// For epoch terminal blocks.  This is both the header state root and the
    /// preseal root.
    ///
    /// (preseal, header)
    Terminal(Buf32, Buf32),
}

impl BlockPostStateCommitments {
    /// Constructs the post-state commitment from a block header and body.
    ///
    /// This exists so we can avoid caring about if we have a full signed block
    /// or not.
    pub fn from_block_parts(header: &OLBlockHeader, body: &OLBlockBody) -> Self {
        if let Some(l1u) = body.l1_update() {
            Self::Terminal(*l1u.preseal_state_root(), *header.state_root())
        } else {
            Self::Common(*header.state_root())
        }
    }

    /// Constructs the post-state commitment from a block.
    pub fn from_block(block: &OLBlock) -> Self {
        Self::from_block_parts(block.header(), block.body())
    }

    /// The state root we check in the header.
    pub fn header_state_root(&self) -> &Buf32 {
        match self {
            BlockPostStateCommitments::Common(r) => r,
            BlockPostStateCommitments::Terminal(_, r) => r,
        }
    }

    /// The "pre-sealing" state root we check in the L1 update.
    pub fn preseal_state_root(&self) -> Option<&Buf32> {
        match self {
            BlockPostStateCommitments::Terminal(r, _) => Some(r),
            _ => None,
        }
    }
}

/// Inputs to block execution, derived from the body.
#[derive(Copy, Clone, Debug)]
pub struct BlockExecInput<'b> {
    tx_segment: &'b OLTxSegment,
    manifest_container: Option<&'b OLL1ManifestContainer>,
}

impl<'b> BlockExecInput<'b> {
    /// Constructs a new instance.
    pub fn new(
        tx_segment: &'b OLTxSegment,
        manifest_container: Option<&'b OLL1ManifestContainer>,
    ) -> Self {
        Self {
            tx_segment,
            manifest_container,
        }
    }

    /// Constructs a new instance by getting refs from a block's body.
    pub fn from_body(body: &'b OLBlockBody) -> Self {
        Self::new(
            body.tx_segment(),
            body.l1_update().map(|u| u.manifest_cont()),
        )
    }

    /// Returns the transaction segment.
    pub fn tx_segment(&self) -> &'b OLTxSegment {
        self.tx_segment
    }

    /// Returns the manifest container if present.
    pub fn manifest_container(&self) -> Option<&'b OLL1ManifestContainer> {
        self.manifest_container
    }

    /// Checks if the body implied by this input is implied to be a terminal block.
    pub fn is_body_terminal(&self) -> bool {
        self.manifest_container().is_some()
    }
}

/// Verifies a block classically by executing it the normal way.
///
/// This closely aligns with `execute_block_inputs`.
pub fn verify_block_classically<S: StateAccessor>(
    state: &mut S,
    header: &OLBlockHeader,
    parent_header: Option<OLBlockHeader>,
    block_exec_input: BlockExecInput<'_>,
    exp: &BlockExecExpectations,
) -> ExecResult<()> {
    // 0. Construct the block exec context for tracking verification state
    // across phases.
    let block_info = BlockInfo::from_header(header);

    // 1. If it's the first block of the epoch, call process_epoch_initial.
    let block_context = BlockContext::new(&block_info, parent_header.as_ref());
    if block_context.is_probably_epoch_initial() {
        let epoch_context = block_context.to_epoch_initial_context();
        chain_processing::process_epoch_initial(state, &epoch_context)?;
    }

    // 2. Process the slot start for every block.
    chain_processing::process_slot_start(state, &block_context)?;

    // 3. Call process_block_tx_segment for every block as usual.
    let output_buffer = ExecOutputBuffer::new_empty();
    let basic_ctx = BasicExecContext::new(block_info, &output_buffer);
    let tx_ctx = TxExecContext::new(&basic_ctx, parent_header.as_ref());
    transaction_processing::process_block_tx_segment(
        state,
        block_exec_input.tx_segment(),
        &tx_ctx,
    )?;

    // 4. Check the state root.
    // - if it's a nonterminal, then check against the header state root
    // - if it *is* a terminal, then check against the preseal state root
    let pre_manifest_state_root = state.compute_state_root()?;

    if block_exec_input.is_body_terminal() {
        // For terminal blocks, check against the preseal state root
        let expected_preseal = exp
            .post_state_roots
            .preseal_state_root()
            .ok_or(ExecError::ChainIntegrity)?;
        if &pre_manifest_state_root != expected_preseal {
            return Err(ExecError::ChainIntegrity);
        }
    } else {
        // For non-terminal blocks, check against the header state root
        if &pre_manifest_state_root != exp.post_state_roots.header_state_root() {
            return Err(ExecError::ChainIntegrity);
        }
    }

    // 5. If it's the last block of an epoch, then call process_block_manifests,
    // then really check the header state root.
    //
    // Then we get the exec output one way or another.
    if let Some(manifest_container) = block_exec_input.manifest_container() {
        manifest_processing::process_block_manifests(
            state,
            manifest_container,
            tx_ctx.basic_context(),
        )?;

        // After processing manifests, check the actual final state root against the header.
        let final_state_root = state.compute_state_root()?;
        if &final_state_root != exp.post_state_roots.header_state_root() {
            return Err(ExecError::ChainIntegrity);
        }
    }

    // 6. Check the logs root.
    let computed_logs_root = compute_logs_root(&output_buffer.into_logs());
    if computed_logs_root != exp.logs_root {
        return Err(ExecError::ChainIntegrity);
    }

    Ok(())
}

/// Helper function to compute logs root.
// TODO move this somewhere?
fn compute_logs_root(logs: &[OLLog]) -> Buf32 {
    if logs.is_empty() {
        return Buf32::zero();
    }

    // Hash each log entry to create leaf nodes.
    let mut leaf_hashes: Vec<[u8; 32]> = logs
        .iter()
        .map(|log| log.compute_hash_commitment().0)
        .collect();

    // BinaryMerkleTree requires power of two leaves.
    let next_power_of_two = leaf_hashes.len().next_power_of_two();
    while leaf_hashes.len() < next_power_of_two {
        leaf_hashes.push([0u8; 32]);
    }

    let tree = BinaryMerkleTree::from_leaves::<Sha256Hasher>(leaf_hashes)
        .expect("power of two leaves should always succeed");

    Buf32(*tree.root())
}

/// Expectations we have about a epoch's processing.
///
/// This is derived from data in the checkpoint. (right?)
#[derive(Clone, Debug)]
pub struct EpochExecExpectations {
    epoch_post_state_root: Buf32,
}

/// Verifies a full-epoch transition, relying on a diff.
///
/// The manifests are expected to be produced synthetically based on what's
/// implied in the checkpoint.
pub fn verify_epoch_with_diff<S: StateAccessor, D: DaScheme<S>>(
    state: &mut S,
    epoch_info: &EpochInfo,
    diff: D::Diff,
    manifests: &OLL1ManifestContainer,
    exp: &EpochExecExpectations,
) -> ExecResult<()> {
    // 1. Apply the initial processing by calling process_epoch_initial.
    let init_ctx = EpochInitialContext::new(epoch_info.epoch(), epoch_info.prev_terminal());
    chain_processing::process_epoch_initial(state, &init_ctx)?;

    // 2. Apply the DA diff.
    D::apply_to_state(diff, state).map_err(|_| ExecError::ChainIntegrity)?;

    // 3. As if it were the last block of an epoch, call process_block_manifests.
    let output = ExecOutputBuffer::new_empty(); // this gets discarded anyways
    let term_ctx = BasicExecContext::new(epoch_info.terminal_info(), &output);
    manifest_processing::process_block_manifests(state, manifests, &term_ctx)?;

    // 4. Verify the final state root.
    let final_state_root = state.compute_state_root()?;
    if final_state_root != exp.epoch_post_state_root {
        return Err(ExecError::ChainIntegrity);
    }

    Ok(())
}
