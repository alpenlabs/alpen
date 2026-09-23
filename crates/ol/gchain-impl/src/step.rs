//! Pieces shared between the OL processor stages.
//!
//! Both stages run the STF over the same pre-state view and classify STF
//! outcomes the same way, so that lives here rather than in either stage.

use std::ops::RangeInclusive;

use strata_asm_common::AsmManifest;
use strata_gchain_types::{PathArtifacts, ProcError};
use strata_identifiers::L1Height;
use strata_ol_chain_types_v1::MAX_SEALING_MANIFEST_COUNT;
use strata_ol_da_types_v1::{OLDaPayloadV1, decode_ol_da_payload_bytes};
use strata_ol_state_support_types::BatchDiffState;
use strata_ol_state_types::{ErrorKind, ExecError, IStateAccessor, StateError};
use strata_ol_state_types_v1::WriteBatch;
use strata_ol_stf_v1::{BlockInfo, EpochInfo};
use thiserror::Error;

use crate::chain_spec::OLChainSpec;
use crate::errors::{InvalidLinkError, OLProcError};
use crate::exec::OLExecArtifact;
use crate::graph_types::{OLCheckpointLink, OLStateNode};
use crate::providers::{L1ManifestProvider, OLStateStore};

/// Why a step didn't produce a valid output.
///
/// The gchain contract is that a stage failing is different from a link being
/// invalid, so the two are kept apart all the way through a step and only
/// merged into the artifact at the end.
#[derive(Debug, Error)]
pub(crate) enum StepError {
    /// The link is invalid.
    #[error(transparent)]
    Invalid(#[from] InvalidLinkError),

    /// A state read failed.
    #[error(transparent)]
    State(#[from] StateError),

    /// The stage couldn't do its work.
    #[error(transparent)]
    Failed(#[from] OLProcError),
}

impl From<StepError> for ProcError {
    fn from(err: StepError) -> Self {
        match err {
            StepError::Invalid(e) => OLProcError::IndexRejectedAcceptedLink(e).into(),
            StepError::State(e) => OLProcError::State(e).into(),
            StepError::Failed(e) => e.into(),
        }
    }
}

/// Classifies an STF error, which only signals invalidity through its kind.
pub(crate) fn exec_step_error(err: ExecError) -> StepError {
    match err.kind() {
        ErrorKind::Correctness => InvalidLinkError::Exec(Box::new(err)).into(),
        ErrorKind::Execution => OLProcError::Exec(Box::new(err)).into(),
    }
}

/// The pre-state for a link: the exec stage's committed state at the base of
/// the path, with the write batches along the path overlaid.
pub(crate) type PreState<'p, 'b, S> =
    BatchDiffState<'p, 'b, <S as OLStateStore>::State, &'p WriteBatch>;

/// Loads the exec stage's committed state at a node.
pub(crate) fn load_base_state<S: OLStateStore>(
    store: &S,
    node: &OLStateNode,
) -> Result<S::State, ProcError> {
    Ok(store
        .fetch_state(node)?
        .ok_or(OLProcError::MissingBaseState(*node))?)
}

/// Borrows the write batches out of the exec artifacts along a path.
///
/// Every artifact on the path has to be valid, since the executor shouldn't
/// have continued past an invalid link.
pub(crate) fn path_write_batches(
    path: &PathArtifacts<OLChainSpec, OLExecArtifact>,
) -> Result<Vec<&WriteBatch>, OLProcError> {
    path.steps()
        .iter()
        .map(|(lref, artifact)| {
            artifact
                .output()
                .map(|o| o.write_batch())
                .ok_or(OLProcError::InvalidLinkOnPath(*lref))
        })
        .collect()
}

/// The inputs a checkpoint step derives from its link and pre-state before
/// running the DA reconstruction.
pub(crate) struct CheckpointInputs {
    pub(crate) da_payload: OLDaPayloadV1,
    pub(crate) manifests: Vec<AsmManifest>,
    pub(crate) epoch_info: EpochInfo,
}

/// Decodes the checkpoint's DA payload and gathers the manifests for the L1
/// range its epoch covers.
pub(crate) fn assemble_checkpoint_inputs(
    manifest_provider: &impl L1ManifestProvider,
    pre_state: &impl IStateAccessor,
    link: &OLCheckpointLink,
) -> Result<CheckpointInputs, StepError> {
    let summary = link.summary();
    let tip = link.payload().new_tip();
    let sidecar = link.payload().sidecar();

    if summary.epoch() == 0 {
        return Err(InvalidLinkError::GenesisEpochCheckpoint.into());
    }

    let da_payload = decode_ol_da_payload_bytes(sidecar.ol_state_diff())
        .map_err(InvalidLinkError::DaPayloadDecode)?;

    let manifests = match epoch_manifest_heights(pre_state.last_l1_height(), tip.l1_height())? {
        Some(heights) => fetch_manifests(manifest_provider, heights)?,
        None => Vec::new(),
    };

    let terminal_info = BlockInfo::new(
        sidecar.terminal_header_complement().timestamp(),
        tip.l2_commitment().slot(),
        summary.epoch(),
    );
    let epoch_info = EpochInfo::new(terminal_info, *summary.prev_terminal());

    Ok(CheckpointInputs {
        da_payload,
        manifests,
        epoch_info,
    })
}

/// The L1 heights an epoch's manifests cover, from just after the pre-state's
/// last seen L1 block up to the checkpoint tip.  `None` when the epoch saw no
/// new L1 blocks.
fn epoch_manifest_heights(
    base: L1Height,
    tip: L1Height,
) -> Result<Option<RangeInclusive<L1Height>>, InvalidLinkError> {
    if tip < base {
        return Err(InvalidLinkError::L1HeightBehind { tip, base });
    }
    if tip == base {
        return Ok(None);
    }

    let len = tip - base;
    let max = MAX_SEALING_MANIFEST_COUNT as u32;
    if len > max {
        return Err(InvalidLinkError::L1RangeTooLong { len, max });
    }

    Ok(Some(base + 1..=tip))
}

fn fetch_manifests(
    provider: &impl L1ManifestProvider,
    heights: RangeInclusive<L1Height>,
) -> Result<Vec<AsmManifest>, OLProcError> {
    heights
        .map(|h| {
            provider
                .fetch_manifest(h)
                .map_err(OLProcError::ManifestFetch)?
                .ok_or(OLProcError::MissingManifest(h))
        })
        .collect()
}
