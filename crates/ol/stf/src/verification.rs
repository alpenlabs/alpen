//! High-level verification and processing flows.
//!
//! These procedures aren't useful for initial execution like during block
//! assembly as they perform all of the block validation checks including
//! outputs (corresponding with headers/checkpoints/etc).

use strata_identifiers::Buf32;
use strata_ledger_types::IStateAccessor;
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

    pub(crate) fn from_block_parts(header: &OLBlockHeader, body: &OLBlockBody) -> Self {
        let psr = BlockPostStateCommitments::from_block_parts(header, body);
        Self::new(psr, *header.logs_root())
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
        // tx_segment is optional in the body, but BlockExecInput requires it.
        // Blocks without transactions should have an empty tx_segment, not None.
        let tx_segment = body
            .tx_segment()
            .expect("block body should have tx_segment for execution");
        Self::new(tx_segment, body.l1_update().map(|u| u.manifest_cont()))
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

/// Verifies a block by executing it the normal way.
///
/// This closely aligns with `execute_block_inputs`.
pub fn verify_block<S: IStateAccessor>(
    state: &mut S,
    header: &OLBlockHeader,
    parent_header: Option<&OLBlockHeader>,
    body: &OLBlockBody,
) -> ExecResult<Vec<OLLog>> {
    // 0. Do preliminary sanity checks.
    verify_header_continuity(header, parent_header)?;
    verify_block_structure(header, body)?;
    let exp = BlockExecExpectations::from_block_parts(header, body);

    // 1. If it's the first block of the epoch, call process_epoch_initial.
    let block_info = BlockInfo::from_header(header);
    let block_context = BlockContext::new(&block_info, parent_header);
    if block_context.is_epoch_initial() {
        let epoch_context = block_context.get_epoch_initial_context();
        chain_processing::process_epoch_initial(state, &epoch_context)?;
    }

    // 2. Process the slot start for every block.
    //
    // This is where we start doing stuff covered by DA.
    chain_processing::process_block_start(state, &block_context)?;

    // 3. Call process_block_tx_segment for every block as usual.
    let output_buffer = ExecOutputBuffer::new_empty();
    let basic_ctx = BasicExecContext::new(block_info, &output_buffer);
    let tx_ctx = TxExecContext::new(&basic_ctx, parent_header);
    if let Some(tx_segment) = body.tx_segment() {
        transaction_processing::process_block_tx_segment(state, tx_segment, &tx_ctx)?;
    }

    // 4. Check the state root.
    // - if it not a terminal, then check against the header state root
    // - if it *is* a terminal, then check against the preseal state root
    let pre_manifest_state_root = state.compute_state_root()?;

    if header.is_terminal() {
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

    // Defense-in-depth: replay execution already enforces emit-time bounds, and
    // this explicit boundary check preserves a verifier-side invariant backstop.
    output_buffer.verify_logs_within_block_limit()?;

    // 6. Check the logs root.
    let logs = output_buffer.into_logs();
    let computed_logs_root = compute_logs_root(&logs);
    if computed_logs_root != exp.logs_root {
        return Err(ExecError::ChainIntegrity);
    }

    Ok(logs)
}

/// Checks that headers are properly continuous and that their fields are
/// logically consistent with each other.
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

        // Check that we're not pretending to be the genesis block.
        if header.epoch() == 0 || header.slot() == 0 {
            return Err(ExecError::ChainIntegrity);
        }

        // Check the epoch matches what we expect it to be based on the previous
        // header.
        let exp_epoch = ph.epoch() + ph.is_terminal() as u32;
        if header.epoch() != exp_epoch {
            // maybe use a different error?
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
            return Err(ExecError::GenesisCoordsNonzero);
        }

        // Do we need this check?
        if !header.parent_blkid().is_null() {
            return Err(ExecError::GenesisParentNonnull);
        }

        // Also we should check that the genesis block is a terminal, I guess
        // that makes sense to do here.
        if !header.is_terminal() {
            return Err(ExecError::GenesisNonterminal);
        }
    }

    Ok(())
}

/// Checks that the block's structure is internally consistent.
pub fn verify_block_structure(header: &OLBlockHeader, body: &OLBlockBody) -> ExecResult<()> {
    // Check that the body matches the field.
    let body_root = body.compute_hash_commitment();
    if body_root != *header.body_root() {
        return Err(ExecError::BlockStructureMismatch);
    }

    // Check that the terminal flag matches if there's an L1 update.
    if body.l1_update().is_some() != header.is_terminal() {
        return Err(ExecError::InconsistentBodyTerminality);
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
pub fn verify_epoch_with_diff<S: IStateAccessor, D: DaScheme<S>>(
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
    output.verify_logs_within_block_limit()?;

    // 4. Verify the final state root.
    let final_state_root = state.compute_state_root()?;
    if final_state_root != exp.epoch_post_state_root {
        return Err(ExecError::ChainIntegrity);
    }

    Ok(())
}

/// Verifies the preseal state root of an epoch using a DA diff only.
///
/// Unlike [`verify_epoch_with_diff`], this does **not** replay manifest
/// processing or check the final post-manifest state root. Use this when
/// manifest processing has already been proven separately (e.g. during
/// block-by-block execution in the checkpoint proof program) to avoid
/// duplicate proving work inside the zkVM guest.
pub fn verify_epoch_preseal_with_diff<S: IStateAccessor, D: DaScheme<S>>(
    state: &mut S,
    epoch_info: &EpochInfo,
    diff: D::Diff,
    expected_preseal_root: &Buf32,
) -> ExecResult<()> {
    // 1. Apply the initial processing by calling process_epoch_initial.
    let init_ctx = EpochInitialContext::new(epoch_info.epoch(), epoch_info.prev_terminal());
    chain_processing::process_epoch_initial(state, &init_ctx)?;

    // 2. Apply the DA diff.
    D::apply_to_state(diff, state).map_err(|_| ExecError::ChainIntegrity)?;

    // 3. Verify the pre-seal state root after applying the DA diff.
    let preseal_state_root = state.compute_state_root()?;
    if &preseal_state_root != expected_preseal_root {
        return Err(ExecError::ChainIntegrity);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use strata_acct_types::BitcoinAmount;
    use strata_codec::{decode_buf_exact, encode_to_vec};
    use strata_identifiers::AccountSerial;
    use strata_ol_chain_types_new::{
        BlockFlags, OLBlockId, OLL1ManifestContainer, OLLog, OLTxSegment,
    };
    use strata_ol_da::{
        AccountInit, AccountTypeInit, GlobalStateDiff, LedgerDiff, NewAccountEntry, OLDaPayloadV1,
        OLDaSchemeV1, StateDiff, U16LenList,
    };
    use strata_ol_state_types::OLState;

    use super::*;
    use crate::{
        assembly::BlockExecOutputs,
        test_utils::{
            create_test_genesis_state, execute_block, genesis_block_components, test_account_id,
        },
    };

    fn test_logs(count: u32) -> Vec<OLLog> {
        (0..count)
            .map(|i| OLLog::new(AccountSerial::from(i), vec![i as u8]))
            .collect()
    }

    fn assembly_logs_root(logs: Vec<OLLog>) -> Buf32 {
        BlockExecOutputs::new(BlockPostStateCommitments::Common(Buf32::zero()), logs)
            .compute_block_logs_root()
    }

    fn setup_epoch1_diff_state() -> (OLState, EpochInfo) {
        let mut state = create_test_genesis_state();
        let genesis_info = BlockInfo::new_genesis(1_000_000);
        let genesis = execute_block(&mut state, &genesis_info, None, genesis_block_components())
            .expect("genesis should execute");
        let terminal_info = BlockInfo::new(1_001_000, 1, state.cur_epoch());
        let epoch_info = EpochInfo::new(terminal_info, genesis.header().compute_block_commitment());
        (state, epoch_info)
    }

    fn state_changing_epoch_diff() -> StateDiff {
        let new_account = NewAccountEntry::new(
            test_account_id(77),
            AccountInit::new(BitcoinAmount::from_sat(1), AccountTypeInit::Empty),
        );
        StateDiff::new(
            GlobalStateDiff::default(),
            LedgerDiff::new(
                U16LenList::new(vec![new_account]),
                U16LenList::new(Vec::new()),
            ),
        )
    }

    fn duplicate_epoch_diff(state_diff: &StateDiff) -> StateDiff {
        let encoded = encode_to_vec(state_diff).expect("state diff should encode");
        decode_buf_exact(&encoded).expect("state diff should decode")
    }

    fn compute_preseal_root_after_epoch_diff(
        state: &OLState,
        epoch_info: &EpochInfo,
        state_diff: StateDiff,
    ) -> Buf32 {
        let mut expected_state = state.clone();
        let init_ctx = EpochInitialContext::new(epoch_info.epoch(), epoch_info.prev_terminal());
        chain_processing::process_epoch_initial(&mut expected_state, &init_ctx)
            .expect("epoch initial processing should succeed");
        OLDaSchemeV1::apply_to_state(OLDaPayloadV1::new(state_diff), &mut expected_state)
            .expect("state-changing epoch diff should apply");
        expected_state
            .compute_state_root()
            .expect("preseal state root should compute")
    }

    #[test]
    fn test_verify_block_structure_happy_path() {
        // Create a body and compute its root
        let tx_segment = OLTxSegment::new(vec![]);
        let body = OLBlockBody::new(
            tx_segment.expect("tx segment should be within limits"),
            None,
        );
        let body_root = body.compute_hash_commitment();

        // Create header with matching body root
        let header = OLBlockHeader::new(
            1000000,
            BlockFlags::zero(),
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
        let body = OLBlockBody::new(
            tx_segment.expect("tx segment should be within limits"),
            None,
        );

        // Create header with wrong body root
        let header = OLBlockHeader::new(
            1000000,
            BlockFlags::zero(),
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

    #[test]
    fn test_verify_epoch_with_diff_final_root_mismatch() {
        let (mut state, epoch_info) = setup_epoch1_diff_state();
        let diff = OLDaPayloadV1::new(state_changing_epoch_diff());
        let manifests = OLL1ManifestContainer::new(vec![]).expect("empty manifests");
        let exp = EpochExecExpectations {
            epoch_post_state_root: Buf32::from([9u8; 32]),
        };

        let res = verify_epoch_with_diff::<_, OLDaSchemeV1>(
            &mut state,
            &epoch_info,
            diff,
            &manifests,
            &exp,
        );
        assert!(matches!(res.unwrap_err(), ExecError::ChainIntegrity));
    }

    #[test]
    fn test_verify_epoch_preseal_with_diff_accepts_matching_root() {
        let (mut state, epoch_info) = setup_epoch1_diff_state();
        let state_diff = state_changing_epoch_diff();
        let expected_preseal_root = compute_preseal_root_after_epoch_diff(
            &state,
            &epoch_info,
            duplicate_epoch_diff(&state_diff),
        );
        let diff = OLDaPayloadV1::new(state_diff);

        verify_epoch_preseal_with_diff::<_, OLDaSchemeV1>(
            &mut state,
            &epoch_info,
            diff,
            &expected_preseal_root,
        )
        .expect("matching preseal root should verify");
    }

    #[test]
    fn test_verify_epoch_preseal_with_diff_rejects_root_mismatch() {
        let (mut state, epoch_info) = setup_epoch1_diff_state();
        let diff = OLDaPayloadV1::new(state_changing_epoch_diff());
        let wrong_preseal_root = Buf32::from([9u8; 32]);

        let res = verify_epoch_preseal_with_diff::<_, OLDaSchemeV1>(
            &mut state,
            &epoch_info,
            diff,
            &wrong_preseal_root,
        );

        assert!(matches!(res.unwrap_err(), ExecError::ChainIntegrity));
    }

    #[test]
    fn test_compute_logs_root_matches_assembly_for_empty_logs() {
        assert_eq!(compute_logs_root(&[]), assembly_logs_root(vec![]));
    }

    #[test]
    fn test_compute_logs_root_matches_assembly_for_padded_log_count() {
        let logs = test_logs(3);
        assert_eq!(compute_logs_root(&logs), assembly_logs_root(logs));
    }
}
