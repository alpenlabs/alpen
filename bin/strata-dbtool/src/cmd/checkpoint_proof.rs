//! Admin commands operating on the checkpoint-proof receipt store.
//!
//! Pairs with `get-checkpoint` / `get-checkpoints-summary` which surface
//! checkpoint payloads; the receipts themselves live in a separate tree
//! (`CheckpointProofSchema`) keyed by [`strata_identifiers::EpochCommitment`].

use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db_types::traits::{CheckpointProofDatabase, DatabaseBackend};
use strata_identifiers::Epoch;

use crate::{
    cli::OutputFormat,
    cmd::checkpoint::get_canonical_epoch_commitment_at,
    output::{
        checkpoint_proof::{CheckpointProofInfo, DeletedCheckpointProofInfo},
        output,
    },
};

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-checkpoint-proof")]
/// Fetch the stored proof receipt for an OL checkpoint epoch.
pub(crate) struct GetCheckpointProofArgs {
    /// checkpoint epoch
    #[argh(positional)]
    pub(crate) epoch: Epoch,

    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "delete-checkpoint-proof")]
/// Delete a stored checkpoint proof receipt for an epoch.
///
/// Operates on the canonical commitment at the given epoch. Use case:
/// force a re-prove after a guest-program upgrade or to drop a stale
/// receipt from a broken run.
pub(crate) struct DeleteCheckpointProofArgs {
    /// checkpoint epoch
    #[argh(positional)]
    pub(crate) epoch: Epoch,

    /// confirm the deletion
    #[argh(switch)]
    pub(crate) confirm: bool,

    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

/// Fetch the proof receipt for a checkpoint epoch.
pub(crate) fn get_checkpoint_proof(
    db: &impl DatabaseBackend,
    args: GetCheckpointProofArgs,
) -> Result<(), DisplayedError> {
    let commitment = get_canonical_epoch_commitment_at(db, args.epoch)?.ok_or_else(|| {
        DisplayedError::UserError(
            "No canonical checkpoint commitment at epoch".to_string(),
            Box::new(args.epoch),
        )
    })?;

    let receipt = db
        .checkpoint_proof_db()
        .get_proof(commitment)
        .internal_error("Failed to read checkpoint proof")?
        .ok_or_else(|| {
            DisplayedError::UserError(
                "No checkpoint proof stored for epoch".to_string(),
                Box::new(args.epoch),
            )
        })?;

    let info = CheckpointProofInfo::from_receipt(args.epoch, *commitment.last_blkid(), &receipt);
    output(&info, args.output_format)
}

/// Delete the proof receipt for a checkpoint epoch.
pub(crate) fn delete_checkpoint_proof(
    db: &impl DatabaseBackend,
    args: DeleteCheckpointProofArgs,
) -> Result<(), DisplayedError> {
    if !args.confirm {
        return Err(DisplayedError::UserError(
            "--confirm is required to delete a checkpoint proof".to_string(),
            Box::new(args.epoch),
        ));
    }

    let commitment = get_canonical_epoch_commitment_at(db, args.epoch)?.ok_or_else(|| {
        DisplayedError::UserError(
            "No canonical checkpoint commitment at epoch".to_string(),
            Box::new(args.epoch),
        )
    })?;

    let existed = db
        .checkpoint_proof_db()
        .del_proof(commitment)
        .internal_error("Failed to delete checkpoint proof")?;

    let ack = DeletedCheckpointProofInfo {
        epoch: args.epoch,
        terminal_blkid: *commitment.last_blkid(),
        existed,
    };
    output(&ack, args.output_format)
}
