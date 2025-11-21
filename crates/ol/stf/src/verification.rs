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
    body: &OLBlockBody,
    exp: &BlockExecExpectations,
) -> ExecResult<()> {
    // 0. Do preliminary sanity checks.
    verify_header_continuity(header, parent_header.as_ref())?;
    // TODO verify_block_structure(...)?;
    // TODO make sure that we properly increment the epoch when the parent was a
    // terminal!

    // 1. If it's the first block of the epoch, call process_epoch_initial.
    let block_info = BlockInfo::from_header(header);
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
    transaction_processing::process_block_tx_segment(state, body.tx_segment(), &tx_ctx)?;

    // 4. Check the state root.
    // - if it's a nonterminal, then check against the header state root
    // - if it *is* a terminal, then check against the preseal state root
    let pre_manifest_state_root = state.compute_state_root()?;

    if body.is_probably_terminal() {
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
    if let Some(manifest_container) = body.l1_update().map(|u| u.manifest_cont()) {
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

/// Checks that headers are properly continuous.
pub fn verify_header_continuity(
    header: &OLBlockHeader,
    parent: Option<&OLBlockHeader>,
) -> ExecResult<()> {
    // Check parent linkages.
    if let Some(ph) = parent {
        // Simply check that the parent block makes sense.
        let pblkid = ph.compute_blkid();
        if *header.parent_blkid() != pblkid {
            return Err(ExecError::BlockParentMismatch);
        }

        // Check epochs don't skip.
        if !((header.epoch() == ph.epoch()) || (header.epoch() == ph.epoch() + 1)) {
            return Err(ExecError::SkipEpochs(ph.epoch(), header.epoch()));
        }

        // Check slots go in order.
        //
        // We're writing this in a weird way to make it easier to handle
        // nonmonotonic slots in future consensus algos.
        let slot_diff = header.slot() as i64 - ph.slot() as i64;
        if slot_diff != 1 {
            return Err(ExecError::SkipTooManySlots(ph.slot(), header.slot()));
        }
    } else {
        // If we don't have a parent, we must be the genesis block.
        if header.slot() != 0 || header.epoch() != 0 {
            return Err(ExecError::NongenesisHeaderMissingParent);
        }

        // Do we need this check?
        if !header.parent_blkid().is_null() {
            return Err(ExecError::GenesisParentNonnull);
        }
    }

    Ok(())
}

/// Checks that the block's structure is internally consistent.
pub fn verify_block_structure(header: &OLBlockHeader, body: &OLBlockBody) -> ExecResult<()> {
    let body_root = body.compute_hash_commitment();
    if body_root != *header.body_root() {
        return Err(ExecError::BlockStructureMismatch);
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

#[cfg(test)]
mod tests {
    use strata_ol_chain_types_new::{OLBlockId, OLL1ManifestContainer, OLL1Update, OLTxSegment};

    use super::*;

    #[test]
    fn test_verify_header_continuity_happy_path() {
        // Test valid genesis
        let genesis = OLBlockHeader::new(
            1000000,
            0,
            0,
            OLBlockId::null(),
            Buf32::zero(),
            Buf32::zero(),
            Buf32::zero(),
        );
        assert!(verify_header_continuity(&genesis, None).is_ok());

        // Test valid parent-child relationship
        let parent = OLBlockHeader::new(
            1000000,
            5,
            1,
            OLBlockId::from(Buf32::from([1u8; 32])),
            Buf32::from([2u8; 32]),
            Buf32::from([3u8; 32]),
            Buf32::from([4u8; 32]),
        );
        let child = OLBlockHeader::new(
            1001000,
            6,
            1,
            parent.compute_blkid(),
            Buf32::from([5u8; 32]),
            Buf32::from([6u8; 32]),
            Buf32::from([7u8; 32]),
        );
        assert!(verify_header_continuity(&child, Some(&parent)).is_ok());
    }

    #[test]
    fn test_verify_header_continuity_failures() {
        // Test wrong parent block ID
        let parent = OLBlockHeader::new(
            1000000,
            5,
            1,
            OLBlockId::from(Buf32::from([1u8; 32])),
            Buf32::zero(),
            Buf32::zero(),
            Buf32::zero(),
        );
        let bad_child = OLBlockHeader::new(
            1001000,
            6,
            1,
            OLBlockId::from(Buf32::from([99u8; 32])), // wrong parent
            Buf32::zero(),
            Buf32::zero(),
            Buf32::zero(),
        );
        assert!(matches!(
            verify_header_continuity(&bad_child, Some(&parent)).unwrap_err(),
            ExecError::BlockParentMismatch
        ));

        // Test epoch skip
        let child_epoch_skip = OLBlockHeader::new(
            1001000,
            6,
            3, // epoch jumps from 1 to 3
            parent.compute_blkid(),
            Buf32::zero(),
            Buf32::zero(),
            Buf32::zero(),
        );
        assert!(matches!(
            verify_header_continuity(&child_epoch_skip, Some(&parent)).unwrap_err(),
            ExecError::SkipEpochs(1, 3)
        ));

        // Test slot skip
        let child_slot_skip = OLBlockHeader::new(
            1001000,
            8,
            1, // slot jumps from 5 to 8
            parent.compute_blkid(),
            Buf32::zero(),
            Buf32::zero(),
            Buf32::zero(),
        );
        assert!(matches!(
            verify_header_continuity(&child_slot_skip, Some(&parent)).unwrap_err(),
            ExecError::SkipTooManySlots(5, 8)
        ));

        // Test non-genesis without parent
        let non_genesis = OLBlockHeader::new(
            1000000,
            1,
            0,
            OLBlockId::null(),
            Buf32::zero(),
            Buf32::zero(),
            Buf32::zero(),
        );
        assert!(matches!(
            verify_header_continuity(&non_genesis, None).unwrap_err(),
            ExecError::NongenesisHeaderMissingParent
        ));

        // Test genesis with non-null parent
        let bad_genesis = OLBlockHeader::new(
            1000000,
            0,
            0,
            OLBlockId::from(Buf32::from([1u8; 32])),
            Buf32::zero(),
            Buf32::zero(),
            Buf32::zero(),
        );
        assert!(matches!(
            verify_header_continuity(&bad_genesis, None).unwrap_err(),
            ExecError::GenesisParentNonnull
        ));
    }

    #[test]
    fn test_verify_block_structure_happy_path() {
        // Create a body and compute its root
        let tx_segment = OLTxSegment::new(vec![]);
        let body = OLBlockBody::new(tx_segment, None);
        let body_root = body.compute_hash_commitment();

        // Create header with matching body root
        let header = OLBlockHeader::new(
            1000000,
            0,
            0,
            OLBlockId::null(),
            body_root,
            Buf32::zero(),
            Buf32::zero(),
        );

        assert!(verify_block_structure(&header, &body).is_ok());
    }

    #[test]
    fn test_verify_block_structure_mismatch() {
        // Create a body
        let tx_segment = OLTxSegment::new(vec![]);
        let body = OLBlockBody::new(tx_segment, None);

        // Create header with wrong body root
        let header = OLBlockHeader::new(
            1000000,
            0,
            0,
            OLBlockId::null(),
            Buf32::from([99u8; 32]), // wrong body root
            Buf32::zero(),
            Buf32::zero(),
        );

        assert!(matches!(
            verify_block_structure(&header, &body).unwrap_err(),
            ExecError::BlockStructureMismatch
        ));
    }






}
