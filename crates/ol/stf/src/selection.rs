//! The spec of the next epoch after a terminal state.
//!
//! The OL switches rules at epoch boundaries, and every node reads the switch
//! from the L1 manifests it already stores: the epoch after one that processed
//! a checkpoint predicate enactment runs the successor of that epoch's spec.
//! No rule set stages or promotes a spec, so the selection is the same for
//! every transition, from V0 to V1 and from each later spec to the next.
//!
//! The ASM decides which checkpoint predicate verifies which L1 range, so no
//! rule set selects the successor spec itself. A node that selects the wrong
//! spec fails rather than diverges: the epoch's state root would not match
//! what the checkpoint commits to.

use std::cmp::Ordering;

use strata_identifiers::{L1BlockCommitment, L1Height};
use strata_ol_chain_types_v1::AsmManifest;
use strata_ol_state_types::{OLSpecId, OLSpecVersions};
use strata_ol_stf_v1::{ExecError, has_checkpoint_predicate_enactment};
use thiserror::Error;

/// The L1 blocks an epoch processed: every block after the last L1 block the
/// previous epoch processed, up to and including the epoch's own last L1 block.
///
/// An epoch that processes no manifests keeps the previous epoch's last L1
/// block, so its range is empty even if that block carries a checkpoint
/// predicate enactment. The genesis epoch's range is empty too.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EpochL1Range {
    prev_last: L1BlockCommitment,
    last: L1BlockCommitment,
}

impl EpochL1Range {
    /// Creates the range after `prev_last`, the last L1 block the previous
    /// epoch processed, up to and including `last`.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidEpochL1Range`] if `last` is below `prev_last`, or at
    /// its height but a different block.
    pub fn new(
        prev_last: L1BlockCommitment,
        last: L1BlockCommitment,
    ) -> Result<Self, InvalidEpochL1Range> {
        let ordered = match last.height().cmp(&prev_last.height()) {
            Ordering::Less => false,
            Ordering::Equal => last == prev_last,
            Ordering::Greater => true,
        };
        if !ordered {
            return Err(InvalidEpochL1Range { prev_last, last });
        }
        Ok(Self { prev_last, last })
    }

    /// Creates the empty range of an epoch that processed no L1 block and so
    /// still ends on `last`, such as the genesis epoch.
    pub fn empty(last: L1BlockCommitment) -> Self {
        Self {
            prev_last: last,
            last,
        }
    }

    /// Returns the last L1 block the previous epoch processed.
    pub fn prev_last(&self) -> &L1BlockCommitment {
        &self.prev_last
    }

    /// Returns the last L1 block processed by the end of the epoch.
    pub fn last(&self) -> &L1BlockCommitment {
        &self.last
    }

    /// Returns the last L1 block the epoch processed itself, or `None` if it
    /// processed none.
    pub fn last_processed(&self) -> Option<&L1BlockCommitment> {
        (self.last.height() > self.prev_last.height()).then_some(&self.last)
    }
}

/// Error returned when an epoch's last L1 block does not follow the previous
/// epoch's.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
#[error("epoch L1 range ends at {last}, which does not follow the previous epoch's {prev_last}")]
pub struct InvalidEpochL1Range {
    prev_last: L1BlockCommitment,
    last: L1BlockCommitment,
}

/// The epoch after a checkpoint predicate enactment runs a spec this binary
/// does not implement.
///
/// A node that gets this must stop instead of running the epoch under older
/// rules: the sequencer builds no block after the terminal block that
/// processed the enactment, and block execution and checkpoint sync apply
/// nothing past that epoch. Running a binary that implements the spec
/// resumes from the stored chain.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
#[error(
    "upgrade required: the checkpoint predicate enactment at L1 height {enactment_l1_height} ends {prev_spec:?}, and this binary does not implement the OL spec version {} that follows it",
    self.spec_version()
)]
pub struct UpgradeRequired {
    prev_spec: OLSpecId,
    enactment_l1_height: L1Height,
}

impl UpgradeRequired {
    /// Returns the spec of the epoch that processed the enactment, the newest
    /// spec this binary implements.
    pub fn prev_spec(&self) -> OLSpecId {
        self.prev_spec
    }

    /// Returns the L1 height of the enactment, the last L1 block the old spec
    /// governs.
    pub fn enactment_l1_height(&self) -> L1Height {
        self.enactment_l1_height
    }

    /// Returns the raw version of the spec the enactment activates.
    ///
    /// Each enactment advances the spec by one, as [`OLSpecId`] describes.
    pub fn spec_version(&self) -> u32 {
        u32::from(self.prev_spec) + 1
    }
}

/// Returns the spec the epoch after a parent epoch runs under.
///
/// `parent_spec` is the parent epoch's spec, and `enactment_l1_height` the
/// height of the checkpoint predicate enactment the parent epoch processed, if
/// it processed one. An enactment activates the successor spec; otherwise the
/// spec carries over.
///
/// # Errors
///
/// Returns [`UpgradeRequired`] if the parent epoch processed an enactment and
/// this binary does not know the successor of `parent_spec`.
pub fn next_epoch_spec(
    parent_spec: OLSpecId,
    enactment_l1_height: Option<L1Height>,
) -> Result<OLSpecId, UpgradeRequired> {
    let Some(enactment_l1_height) = enactment_l1_height else {
        return Ok(parent_spec);
    };
    parent_spec.successor().ok_or(UpgradeRequired {
        prev_spec: parent_spec,
        enactment_l1_height,
    })
}

/// Errors returned by [`select_next_epoch_spec`].
#[derive(Debug, Error)]
pub enum EpochSpecSelectionError<E> {
    /// Looking up the stored L1 manifest failed.
    #[error("look up the L1 manifest of {block}: {source}")]
    ManifestLookup {
        block: L1BlockCommitment,
        #[source]
        source: E,
    },

    /// No L1 manifest is stored for the last L1 block the parent epoch
    /// processed.
    #[error("missing the L1 manifest of {block}, the last L1 block the parent epoch processed")]
    MissingLastManifest { block: L1BlockCommitment },

    /// The stored L1 manifest is for another L1 block than the last one the
    /// parent epoch processed.
    #[error("stored L1 manifest is for {found}, but the parent epoch processed {expected} last")]
    LastManifestMismatch {
        expected: L1BlockCommitment,
        found: L1BlockCommitment,
    },

    /// The manifest carries a malformed or duplicated checkpoint predicate
    /// enactment.
    #[error("select the next epoch's spec: {0}")]
    Exec(#[from] ExecError),

    /// The next epoch runs a spec this binary does not implement.
    #[error(transparent)]
    UpgradeRequired(#[from] UpgradeRequired),
}

/// Selects the spec of the epoch after a parent epoch, with
/// [`next_epoch_spec`].
///
/// `parent_versions` are the spec versions of the parent epoch's terminal
/// state. Its current spec is the parent epoch's spec: every rule set leaves
/// the state's current spec at its own identifier from the first block of its
/// epoch on, as V1 does by wrapping a V0 state. The staged version is never
/// read; no rule set stages a spec.
///
/// `parent_l1_range` is the range of L1 blocks the parent epoch processed.
/// Only an enactment the parent epoch processed itself counts. A checkpoint
/// predicate enactment always ends the epoch that processes it: V1 requires it
/// to be the last manifest of a terminal block, and the ASM accepts no
/// checkpoint whose L1 range crosses one. So the parent epoch processed an
/// enactment exactly when its range is not empty and its last L1 block's
/// manifest carries one. An epoch that processed no manifests still ends on
/// the previous epoch's last L1 block, which may carry the enactment that
/// started the parent epoch's spec; its empty range keeps that enactment from
/// counting twice.
///
/// `manifest_for` returns the stored manifest of an L1 block, or `None` if
/// none is stored. It is called only for a non-empty range, and the manifest
/// must be for exactly the parent's last L1 block, so a reorged manifest is
/// never read as the parent's.
///
/// # Errors
///
/// Returns an error if the lookup fails, the manifest is missing or for
/// another L1 block, the manifest carries a malformed or duplicated
/// enactment, or [`next_epoch_spec`] reports [`UpgradeRequired`].
pub fn select_next_epoch_spec<E>(
    parent_versions: OLSpecVersions,
    parent_l1_range: EpochL1Range,
    manifest_for: impl FnOnce(&L1BlockCommitment) -> Result<Option<AsmManifest>, E>,
) -> Result<OLSpecId, EpochSpecSelectionError<E>> {
    let enactment_l1_height = match parent_l1_range.last_processed() {
        None => None,
        Some(&block) => {
            let manifest = manifest_for(&block)
                .map_err(|source| EpochSpecSelectionError::ManifestLookup { block, source })?
                .ok_or(EpochSpecSelectionError::MissingLastManifest { block })?;
            let found = L1BlockCommitment::new(manifest.height(), *manifest.blkid());
            if found != block {
                return Err(EpochSpecSelectionError::LastManifestMismatch {
                    expected: block,
                    found,
                });
            }
            has_checkpoint_predicate_enactment(&manifest)?.then_some(block.height())
        }
    };
    Ok(next_epoch_spec(
        parent_versions.cur_spec(),
        enactment_l1_height,
    )?)
}
