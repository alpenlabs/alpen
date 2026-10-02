//! Checkpoint status derived from persisted signing and L1-observation facts.

use strata_csm_types::CheckpointL1Ref;
use strata_identifiers::{Epoch, EpochCommitment};
use strata_primitives::L1Height;

use crate::common::L1PayloadIntentIndex;
use crate::ol_checkpoint::OLCheckpointDatabase;
use crate::DbResult;

/// Persisted facts needed to derive one checkpoint's operator-facing status.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckpointStatusRecord {
    /// Signing intent, when the checkpoint has been signed.
    pub signing: Option<L1PayloadIntentIndex>,
    /// Canonical L1 observation, when the checkpoint has been published.
    pub l1_ref: Option<CheckpointL1Ref>,
}

/// Operator-facing checkpoint publication and finality state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckpointStatus {
    /// No signing intent or L1 observation exists.
    Unsigned,
    /// A signing intent exists but no L1 observation exists.
    Signed,
    /// An L1 observation exists below the configured reorg-safe depth.
    Confirmed,
    /// An L1 observation has reached the configured reorg-safe depth.
    Finalized,
}

impl CheckpointStatus {
    /// Returns the stable display name used by database diagnostics.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unsigned => "Unsigned",
            Self::Signed => "Signed",
            Self::Confirmed => "Confirmed",
            Self::Finalized => "Finalized",
        }
    }
}

/// Derives checkpoint state using the same one-confirmation minimum used by the node tools.
pub fn derive_checkpoint_status(
    status_record: CheckpointStatusRecord,
    current_l1_tip: L1Height,
    l1_reorg_safe_depth: u32,
) -> CheckpointStatus {
    match (status_record.signing, status_record.l1_ref) {
        (None, None) => CheckpointStatus::Unsigned,
        (Some(_), None) => CheckpointStatus::Signed,
        (_, Some(l1_ref)) => {
            let confirmations = current_l1_tip
                .saturating_sub(l1_ref.l1_commitment.height())
                .saturating_add(1);
            if confirmations >= l1_reorg_safe_depth.max(1) {
                CheckpointStatus::Finalized
            } else {
                CheckpointStatus::Confirmed
            }
        }
    }
}

/// Resolves the commitment used by existing diagnostics for an epoch.
pub fn read_canonical_epoch_commitment(
    database: &impl OLCheckpointDatabase,
    epoch: Epoch,
) -> DbResult<Option<EpochCommitment>> {
    if epoch == 0 {
        return Ok(None);
    }
    database
        .get_epoch_commitments_at(epoch)
        .map(|commitments| commitments.first().copied())
}

/// Derives one stored checkpoint's status from its persisted facts.
pub fn read_checkpoint_status_by_commitment(
    database: &impl OLCheckpointDatabase,
    commitment: EpochCommitment,
    current_l1_tip: L1Height,
    l1_reorg_safe_depth: u32,
) -> DbResult<Option<CheckpointStatus>> {
    if database.get_checkpoint_payload_entry(commitment)?.is_none() {
        return Ok(None);
    }
    let signing = database.get_checkpoint_signing_entry(commitment)?;
    let l1_ref = database.get_checkpoint_l1_ref(commitment)?;
    Ok(Some(derive_checkpoint_status(
        CheckpointStatusRecord { signing, l1_ref },
        current_l1_tip,
        l1_reorg_safe_depth,
    )))
}

/// Resolves an epoch and derives its stored checkpoint status.
pub fn read_checkpoint_status(
    database: &impl OLCheckpointDatabase,
    epoch: Epoch,
    current_l1_tip: L1Height,
    l1_reorg_safe_depth: u32,
) -> DbResult<Option<CheckpointStatus>> {
    let Some(commitment) = read_canonical_epoch_commitment(database, epoch)? else {
        return Ok(None);
    };
    read_checkpoint_status_by_commitment(database, commitment, current_l1_tip, l1_reorg_safe_depth)
}

/// Returns the latest stored checkpoint at the configured reorg-safe depth.
pub fn read_latest_finalized_checkpoint_epoch(
    database: &impl OLCheckpointDatabase,
    current_l1_tip: L1Height,
    l1_reorg_safe_depth: u32,
) -> DbResult<Option<EpochCommitment>> {
    let Some(last_epoch) = database
        .get_last_checkpoint_payload_epoch()?
        .map(|commitment| commitment.epoch())
    else {
        return Ok(None);
    };

    for epoch in (1..=last_epoch).rev() {
        if read_checkpoint_status(database, epoch, current_l1_tip, l1_reorg_safe_depth)?
            == Some(CheckpointStatus::Finalized)
        {
            return read_canonical_epoch_commitment(database, epoch);
        }
    }
    Ok(None)
}
