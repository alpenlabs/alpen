//! High-level verification and processing flows.
//!
//! These procedures aren't useful for initial execution like during block
//! assembly as they perform all of the block validation checks including
//! outputs (corresponding with headers/checkpoints/etc).

use strata_identifiers::Buf32;
use strata_ledger_types::StateAccessor;
use strata_ol_chain_types_new::{
    OLBlock, OLBlockBody, OLBlockHeader, OLL1ManifestContainer, OLTxSegment,
};
use strata_ol_da::DaScheme;

use crate::errors::{ExecError, ExecResult};

/// Commitments that we are checking against a block.
///
/// This is ultimately derived from the header (and maybe L1 update data).
#[derive(Copy, Clone, Debug)]
pub struct BlockExecExpectations {
    post_state_roots: BlockPostStateCommitments,
    logs_root: Buf32,
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
}

/// Verifies a block classically by executing it the normal way.
///
/// This closely aligns with `execute_block_inputs`.
pub fn verify_block_classically<S: StateAccessor>(
    state: &mut S,
    header: &OLBlockHeader,
    block_exec_input: BlockExecInput<'_>,
    exp: &BlockExecExpectations,
) -> ExecResult<()> {
    // 0. Construct the block exec context for tracking verification state
    // across phases.
    // TODO

    // 1. If it's the first block of the epoch, call process_epoch_initial.
    // TODO

    // 2. Call process_block_tx_segment for every block as usual.
    // TODO

    // 3. Check the state root.
    // - if it's a nonterminal, then check against the header state root
    // - if it *is* a terminal, then check against the preseal state root
    // TODO

    // 4. If it's the last block of an epoch, then call process_block_manifests, then really check
    //    the header state root.
    // TODO

    // 5. Check the logs root.
    // TODO

    Ok(())
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
    diff: D::Diff,
    manifests: &OLL1ManifestContainer,
    exp: &EpochExecExpectations,
) -> ExecResult<()> {
    // 1. Apply the initial processing by calling process_epoch_initial.
    // TODO

    // 2. Apply the DA diff.
    // TODO

    // 3. As if it were the last block of an epoch, call process_block_manifests.
    // TODO

    Ok(())
}
