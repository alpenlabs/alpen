use strata_acct_types::{AccountId, AccountSerial};
use strata_checkpoint_types::TerminalHeaderReconstructionError;
use strata_codec::CodecError;
use strata_gchain_types::{ProcError, ProcId};
use strata_identifiers::{Buf32, L1Height};
use strata_ol_chain_types_v1::LogDecodeError;
use strata_ol_state_types::{ExecError, StateError};
use strata_snark_acct_types::Seqno;
use thiserror::Error;

use crate::graph_types::{OLLinkRef, OLStateNode};

/// Failures specific to the OL processor stages.
///
/// These all mean a stage *couldn't do its work*; a link being invalid is
/// reported through the stage's artifact instead (see [`InvalidLinkError`]).
/// They're carried inside [`ProcError::Custom`] so the executor sees them as
/// ordinary stage failures.
#[derive(Debug, Error)]
pub enum OLProcError {
    /// The stage's committed state at its base node isn't in the store, so
    /// there's nothing to build the pre-state on.
    #[error("missing committed state at node {0:?}")]
    MissingBaseState(OLStateNode),

    /// The context couldn't supply the artifacts along the path from the
    /// committed node, which the stage needs to reconstruct the pre-state.
    #[error("missing path artifacts from proc {0}")]
    MissingPathArtifacts(ProcId),

    /// The executor extended the path through a link a stage had already
    /// found invalid, which it should never do.
    #[error("path continues through invalid link {0:?}")]
    InvalidLinkOnPath(OLLinkRef),

    /// The state a commit produced doesn't have the root the node it was
    /// committed to claims.
    #[error("committed state root mismatch at {node:?} (got {got})")]
    StateRootMismatch { node: OLStateNode, got: Buf32 },

    /// A manifest a checkpoint's epoch covers isn't available yet.
    #[error("missing L1 manifest at height {0}")]
    MissingManifest(L1Height),

    /// The manifest provider failed.
    #[error("fetching L1 manifest failed: {0}")]
    ManifestFetch(#[source] ProcError),

    /// The index stage found a link invalid that the exec stage had accepted,
    /// so the two disagree about the STF.
    #[error("index stage rejected link exec stage accepted: {0}")]
    IndexRejectedAcceptedLink(#[source] InvalidLinkError),

    /// A state read or write failed.
    #[error("state access failed: {0}")]
    State(#[from] StateError),

    /// The STF failed for a reason unrelated to the link's validity.
    #[error("execution failed: {0}")]
    Exec(#[source] Box<ExecError>),
}

impl From<OLProcError> for ProcError {
    fn from(err: OLProcError) -> Self {
        ProcError::custom(err)
    }
}

/// Reasons a stage rejects a link.
///
/// This is the stage's verdict on the link, not a failure: it's recorded in
/// the stage's artifact so the executor can steer around the link.
#[derive(Debug, Error)]
pub enum InvalidLinkError {
    /// The STF rejected the block or the DA reconstruction.
    #[error("STF rejected link: {0}")]
    Exec(Box<ExecError>),

    /// Epoch 0 is genesis-initialized on every node, never checkpoint-applied.
    #[error("checkpoint for genesis epoch")]
    GenesisEpochCheckpoint,

    #[error("undecodable DA payload: {0}")]
    DaPayloadDecode(#[from] CodecError),

    /// The checkpoint claims an L1 tip before the one the pre-state already
    /// reached.
    #[error("checkpoint L1 height behind pre-state (tip {tip}, base {base})")]
    L1HeightBehind { tip: L1Height, base: L1Height },

    /// The checkpoint's epoch spans more L1 blocks than an epoch may seal.
    #[error("checkpoint L1 range too long (got {len}, max {max})")]
    L1RangeTooLong { len: u32, max: u32 },

    #[error("terminal header reconstruction failed: {0}")]
    TerminalHeader(#[from] TerminalHeaderReconstructionError),

    #[error("malformed log payload: {0}")]
    MalformedLog(#[from] strata_msg_fmt::Error),

    #[error("undecodable snark update log: {0}")]
    SnarkUpdateLogDecode(#[from] LogDecodeError),

    /// A checkpoint log names an account serial the reconstructed state
    /// doesn't have.
    #[error("log for unknown account serial {0:?}")]
    UnknownAccountSerial(AccountSerial),

    /// The number of updates the checkpoint's logs record for an account
    /// disagrees with the seqno its DA diff produced.
    #[error(
        "snark seqno for account {account_id} disagrees between logs and diff \
         (logs {logs:?}, diff {diff:?})"
    )]
    SnarkSeqnoMismatch {
        account_id: AccountId,
        logs: Seqno,
        diff: Seqno,
    },
}
