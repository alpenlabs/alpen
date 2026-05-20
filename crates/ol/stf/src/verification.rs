//! High-level verification and processing flows.
//!
//! These procedures aren't useful for initial execution like during block
//! assembly as they perform all of the block validation checks including
//! outputs (corresponding with headers/checkpoints/etc).

use strata_bridge_params::BridgeParams;
use strata_identifiers::Buf32;
use strata_ledger_types::*;
use strata_merkle::{BinaryMerkleTree, Sha256Hasher};
use strata_ol_chain_types_new::{
    AsmManifest, MAX_LOGS_PER_BLOCK, OLAsmManifestContainer, OLBlockBody, OLBlockHeader, OLLog,
    OLTxSegment,
};
use strata_ol_da::DaScheme;
use tracing::error;

use crate::{
    chain_processing,
    context::*,
    errors::{ExecError, ExecResult},
    manifest_processing,
    output::ExecOutputBuffer,
    transaction_processing,
};

/// Commitments that we are checking against a block.
///
/// This is ultimately derived from the header.
#[derive(Copy, Clone, Debug)]
pub struct BlockExecExpectations {
    state_root: Buf32,
    logs_root: Buf32,
}

impl BlockExecExpectations {
    pub(crate) fn new(state_root: Buf32, logs_root: Buf32) -> Self {
        Self {
            state_root,
            logs_root,
        }
    }

    pub(crate) fn from_block_parts(header: &OLBlockHeader, _body: &OLBlockBody) -> Self {
        Self::new(*header.state_root(), *header.logs_root())
    }

    /// The single final state root committed in the header.
    pub(crate) fn state_root(&self) -> &Buf32 {
        &self.state_root
    }
}

/// Inputs to block execution, derived from the header and body.
#[derive(Copy, Clone, Debug)]
pub struct BlockExecInput<'b> {
    tx_segment: &'b OLTxSegment,
    manifest_container: Option<&'b OLAsmManifestContainer>,
    is_terminal: bool,
}

impl<'b> BlockExecInput<'b> {
    /// Constructs a new instance.
    pub fn new(
        tx_segment: &'b OLTxSegment,
        manifest_container: Option<&'b OLAsmManifestContainer>,
        is_terminal: bool,
    ) -> Self {
        Self {
            tx_segment,
            manifest_container,
            is_terminal,
        }
    }

    /// Constructs a new instance from a block's header and body.
    ///
    /// Terminality is read from the authoritative `IS_TERMINAL` header flag.
    pub fn from_block_parts(header: &OLBlockHeader, body: &'b OLBlockBody) -> Self {
        // tx_segment is optional in the body, but BlockExecInput requires it.
        // Blocks without transactions should have an empty tx_segment, not None.
        let tx_segment = body
            .tx_segment()
            .expect("block body should have tx_segment for execution");
        Self::new(tx_segment, body.manifests(), header.is_terminal())
    }

    /// Returns the transaction segment.
    pub fn tx_segment(&self) -> &'b OLTxSegment {
        self.tx_segment
    }

    /// Returns the manifest container if present.
    pub fn manifest_container(&self) -> Option<&'b OLAsmManifestContainer> {
        self.manifest_container
    }

    /// Whether this block is the epoch terminal.
    pub fn is_terminal(&self) -> bool {
        self.is_terminal
    }
}

/// Verifies a block end-to-end. Composes [`verify_block_predrain`] and
/// [`apply_epoch_terminal`].
#[tracing::instrument(
    skip_all,
    fields(
        slot = header.slot(),
        epoch = header.epoch(),
        is_terminal = header.is_terminal(),
        tx_count = body.tx_segment().map(|s| s.txs().len()).unwrap_or(0),
    ),
)]
pub fn verify_block<S: IStateAccessorMut>(
    state: &mut S,
    header: &OLBlockHeader,
    parent_header: Option<&OLBlockHeader>,
    body: &OLBlockBody,
    bridge_params: BridgeParams,
) -> ExecResult<Vec<OLLog>> {
    let exp = BlockExecExpectations::from_block_parts(header, body);

    let mut logs = verify_block_predrain(state, header, parent_header, body, bridge_params)?;
    logs.extend(apply_epoch_terminal(state, header, body)?);

    // Verify logs size.
    let max = MAX_LOGS_PER_BLOCK as usize;
    if logs.len() > max {
        return Err(ExecError::LogsOverflow {
            count: logs.len(),
            max,
        });
    }

    // Verify logs root.
    if compute_logs_root(&logs) != exp.logs_root {
        return Err(ExecError::ChainIntegrity);
    }

    Ok(logs)
}

/// Runs the pre-drain stages of block verification (tx segment + manifest
/// buffering) and, for non-terminal blocks, the state root check. Returns the
/// tx-segment logs. Pair with [`apply_epoch_terminal`] for full verification.
///
/// Terminal blocks defer their state-root check to [`apply_epoch_terminal`],
/// since the single committed header root reflects the post-drain state.
#[tracing::instrument(
    skip_all,
    fields(
        slot = header.slot(),
        epoch = header.epoch(),
        is_terminal = header.is_terminal(),
        tx_count = body.tx_segment().map(|s| s.txs().len()).unwrap_or(0),
    ),
)]
pub fn verify_block_predrain<S: IStateAccessorMut>(
    state: &mut S,
    header: &OLBlockHeader,
    parent_header: Option<&OLBlockHeader>,
    body: &OLBlockBody,
    bridge_params: BridgeParams,
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
    let basic_ctx =
        BasicExecContext::new(block_info, &output_buffer).with_bridge_params(bridge_params);
    let tx_ctx = TxExecContext::new(&basic_ctx, parent_header);
    if let Some(tx_segment) = body.tx_segment() {
        transaction_processing::process_block_tx_segment(state, tx_segment, &tx_ctx)?;
    }

    // 4. Buffer any manifests carried by this block (allowed in any block).
    if let Some(manifest_container) = body.manifests() {
        manifest_processing::process_block_manifests(state, manifest_container.manifests())?;
    }

    // 5. For non-terminal blocks, the header state root reflects the state
    // after buffering, so check it now. Terminal blocks check the root after
    // the drain in `apply_epoch_terminal`.
    if !header.is_terminal() {
        let computed_root = state.compute_state_root()?;
        if &computed_root != exp.state_root() {
            error!(
                computed = %computed_root,
                expected = %exp.state_root(),
                slot = header.slot(),
                epoch = header.epoch(),
                check = "state_root",
                "non-terminal block state root mismatch"
            );
            return Err(ExecError::ChainIntegrity);
        }
    }

    Ok(output_buffer.into_logs())
}

/// Runs the epoch-terminal drain (for terminal blocks) and the post-drain
/// state root check against the header. Returns the drain-emitted logs.
///
/// This is a no-op for non-terminal blocks.
pub fn apply_epoch_terminal<S: IStateAccessorMut>(
    state: &mut S,
    header: &OLBlockHeader,
    body: &OLBlockBody,
) -> ExecResult<Vec<OLLog>> {
    let exp = BlockExecExpectations::from_block_parts(header, body);
    let block_info = BlockInfo::from_header(header);

    let output_buffer = ExecOutputBuffer::new_empty();
    let basic_ctx = BasicExecContext::new(block_info, &output_buffer);

    // If this is the epoch terminal, drain the buffered ASM logs, reset
    // intraepoch state, advance the epoch, then check the header state root.
    if header.is_terminal() {
        manifest_processing::process_epoch_terminal(state, &basic_ctx)?;

        let final_state_root = state.compute_state_root()?;
        if &final_state_root != exp.state_root() {
            error!(
                computed = %final_state_root,
                expected = %exp.state_root(),
                slot = header.slot(),
                epoch = header.epoch(),
                check = "terminal_state_root",
                "terminal block state root mismatch"
            );
            return Err(ExecError::ChainIntegrity);
        }
    }

    Ok(output_buffer.into_logs())
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

    // Terminality is signalled authoritatively by the header `IS_TERMINAL`
    // flag and is independent of whether the body carries manifests, so there
    // is no body/terminal consistency check here.

    Ok(())
}

/// Helper function to compute logs root.
// TODO(STR-3677): move this somewhere?
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

impl EpochExecExpectations {
    /// Constructs expectations from the epoch's final post-state root.
    pub fn new(epoch_post_state_root: Buf32) -> Self {
        Self {
            epoch_post_state_root,
        }
    }
}

/// Verifies a full-epoch transition, relying on a diff.
///
/// The manifests are expected to be produced synthetically based on what's
/// implied in the checkpoint.
pub fn verify_epoch_with_diff<S: IStateAccessorMut, D: DaScheme<S>>(
    state: &mut S,
    epoch_info: &EpochInfo,
    diff: D::Diff,
    manifests: &[AsmManifest],
    exp: &EpochExecExpectations,
) -> ExecResult<()> {
    apply_da_epoch::<S, D>(state, epoch_info, diff, manifests)?;

    let final_state_root = state.compute_state_root()?;
    if final_state_root != exp.epoch_post_state_root {
        error!(
            computed = %final_state_root,
            expected = %exp.epoch_post_state_root,
            epoch = epoch_info.epoch(),
            check = "epoch_post_state_root",
            "epoch post-state root mismatch"
        );
        return Err(ExecError::ChainIntegrity);
    }

    Ok(())
}

/// Reconstructs a full-epoch transition from a DA diff.
///
/// Like [`verify_epoch_with_diff`] but without the post-state root check. Use this when post root
/// check is not needed i.e. the diff is trusted.
pub fn apply_da_epoch<S: IStateAccessorMut, D: DaScheme<S>>(
    state: &mut S,
    epoch_info: &EpochInfo,
    diff: D::Diff,
    manifests: &[AsmManifest],
) -> ExecResult<()> {
    let init_ctx = EpochInitialContext::new(epoch_info.epoch(), epoch_info.prev_terminal());
    chain_processing::process_epoch_initial(state, &init_ctx)?;

    D::apply_to_state(diff, state).map_err(|e| {
        error!(
            error = %e,
            epoch = epoch_info.epoch(),
            "DA scheme failed to apply diff during epoch reconstruction"
        );
        ExecError::ChainIntegrity
    })?;

    // As if it were the epoch terminal, replay the manifest buffering and
    // then drain the buffered logs. The DA diff (step 2) reproduces the
    // ledger/global tx effects; replaying the manifests here reproduces the
    // intraepoch/MMR/epochal state and the deferred drain effects, which the
    // DA diff does not carry.
    let output = ExecOutputBuffer::new_empty(); // this gets discarded anyways
    let term_ctx = BasicExecContext::new(epoch_info.terminal_info(), &output);
    manifest_processing::process_block_manifests(state, manifests)?;
    manifest_processing::process_epoch_terminal(state, &term_ctx)?;
    output.verify_logs_within_block_limit()?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use strata_acct_types::BitcoinAmount;
    use strata_codec::{decode_buf_exact, encode_to_vec};
    use strata_identifiers::AccountSerial;
    use strata_ol_chain_types_new::{
        BlockFlags, OLAsmManifestContainer, OLBlockId, OLLog, OLTxSegment,
    };
    use strata_ol_da::{
        AccountInit, AccountTypeInit, GlobalStateDiff, LedgerDiff, NewAccountEntry, OLDaPayloadV1,
        OLDaSchemeV1, StateDiff, U16LenList,
    };
    use strata_ol_state_support_types::MemoryStateBaseLayer;

    use super::*;
    use crate::{
        assembly::BlockExecOutputs,
        test_utils::{FixtureAsmManifestBuilder, OLStfFixture, make_account_id},
    };

    const STATE_DIFF_EMPTY_ACCOUNT_ID: u32 = 77;

    fn make_sequential_logs(count: u32) -> Vec<OLLog> {
        (0..count)
            .map(|i| OLLog::new(AccountSerial::from(i), vec![i as u8]))
            .collect()
    }

    fn compute_assembly_logs_root(logs: Vec<OLLog>) -> Buf32 {
        BlockExecOutputs::new(Buf32::zero(), logs).compute_block_logs_root()
    }

    fn setup_epoch1_diff_state() -> (MemoryStateBaseLayer, EpochInfo) {
        let fixture = OLStfFixture::builder().execute_genesis();
        let state = fixture.state().clone();
        let terminal_info = BlockInfo::new(1_001_000, 1, state.cur_epoch());
        let epoch_info = EpochInfo::new(
            terminal_info,
            fixture.parent_header().compute_block_commitment(),
        );
        (state, epoch_info)
    }

    fn state_changing_epoch_diff() -> StateDiff {
        let new_empty_acct_id = make_account_id(STATE_DIFF_EMPTY_ACCOUNT_ID);
        let new_account = NewAccountEntry::new(
            new_empty_acct_id,
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

    fn compute_post_epoch_root_after_diff(
        state: &MemoryStateBaseLayer,
        epoch_info: &EpochInfo,
        state_diff: StateDiff,
        manifests: &OLAsmManifestContainer,
    ) -> Buf32 {
        let mut expected_state = state.clone();
        let init_ctx = EpochInitialContext::new(epoch_info.epoch(), epoch_info.prev_terminal());
        chain_processing::process_epoch_initial(&mut expected_state, &init_ctx)
            .expect("epoch initial processing should succeed");
        OLDaSchemeV1::apply_to_state(OLDaPayloadV1::new(state_diff), &mut expected_state)
            .expect("state-changing epoch diff should apply");
        let output = ExecOutputBuffer::new_empty();
        let term_ctx = BasicExecContext::new(epoch_info.terminal_info(), &output);
        manifest_processing::process_block_manifests(&mut expected_state, manifests.manifests())
            .expect("manifest buffering should succeed");
        manifest_processing::process_epoch_terminal(&mut expected_state, &term_ctx)
            .expect("epoch terminal processing should succeed");
        expected_state
            .compute_state_root()
            .expect("post-epoch state root should compute")
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
            verify_block_structure(&header, &body)
                .expect_err("mismatched body root should fail structure verification"),
            ExecError::BlockStructureMismatch
        ));
    }

    #[test]
    fn test_verify_block_structure_accepts_terminal_header_without_manifests() {
        // Terminality is now signalled by the header flag and is independent of
        // manifest presence, so a terminal block carrying no manifests is
        // structurally valid.
        let tx_segment = OLTxSegment::new(vec![]).expect("tx segment should be within limits");
        let body = OLBlockBody::new(tx_segment, None);
        let body_root = body.compute_hash_commitment();

        let mut flags = BlockFlags::zero();
        flags.set_is_terminal(true);
        let header = OLBlockHeader::new(
            1000000,
            flags,
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
    fn test_verify_block_structure_accepts_nonterminal_header_with_manifests() {
        // Manifests may be included in any block, including non-terminal ones.
        let tx_segment = OLTxSegment::new(vec![]).expect("tx segment should be within limits");
        let manifests = OLAsmManifestContainer::new(vec![]).expect("empty manifests should fit");
        let body = OLBlockBody::new(tx_segment, Some(manifests));
        let body_root = body.compute_hash_commitment();

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
    fn test_verify_epoch_with_diff_final_root_mismatch() {
        let (mut state, epoch_info) = setup_epoch1_diff_state();
        let diff = OLDaPayloadV1::new(state_changing_epoch_diff());
        let manifests = OLAsmManifestContainer::new(vec![]).expect("empty manifests");
        let exp = EpochExecExpectations {
            epoch_post_state_root: Buf32::from([9u8; 32]),
        };

        let res = verify_epoch_with_diff::<_, OLDaSchemeV1>(
            &mut state,
            &epoch_info,
            diff,
            manifests.manifests(),
            &exp,
        );
        assert!(matches!(
            res.expect_err("mismatched final root should fail epoch verification"),
            ExecError::ChainIntegrity
        ));
    }

    #[test]
    fn test_verify_epoch_with_diff_accepts_matching_root() {
        // Non-empty manifest at the next L1 height (no logs) so the replayed
        // `buffer_block_manifests` advances `last_l1_height` and updates the
        // asm-manifests MMR, and `process_epoch_terminal` advances the epoch.
        // Per-log behavior is covered in `asm_manifests.rs`.
        let (mut state, epoch_info) = setup_epoch1_diff_state();
        let state_diff = state_changing_epoch_diff();
        let next_manifest = FixtureAsmManifestBuilder::new_at_height(state.last_l1_height() + 1)
            .with_variant(2)
            .build();
        let manifests = OLAsmManifestContainer::new(vec![next_manifest])
            .expect("manifest container should fit");

        let expected_post_root = compute_post_epoch_root_after_diff(
            &state,
            &epoch_info,
            duplicate_epoch_diff(&state_diff),
            &manifests,
        );
        let exp = EpochExecExpectations {
            epoch_post_state_root: expected_post_root,
        };

        verify_epoch_with_diff::<_, OLDaSchemeV1>(
            &mut state,
            &epoch_info,
            OLDaPayloadV1::new(state_diff),
            manifests.manifests(),
            &exp,
        )
        .expect("matching post-epoch root should verify");
    }

    #[test]
    fn test_compute_logs_root_matches_assembly_for_empty_logs() {
        assert_eq!(compute_logs_root(&[]), compute_assembly_logs_root(vec![]));
    }

    #[test]
    fn test_compute_logs_root_matches_assembly_for_padded_log_count() {
        let logs = make_sequential_logs(3);
        assert_eq!(compute_logs_root(&logs), compute_assembly_logs_root(logs));
    }
}
