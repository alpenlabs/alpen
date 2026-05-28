//! Error types for EE DA proof verification.

use alpen_ee_da_types::{DaParseError, EvmHeaderSummary};
use alpen_reth_statediff::ReconstructError;
use strata_codec::CodecError;

/// Errors raised while verifying EE DA witness data.
#[derive(Debug, thiserror::Error)]
pub enum DaVerificationError {
    #[error("batch under proof has chunks but no DA witness blocks")]
    MissingDaWitness,
    #[error("batch under proof has no chunks; cannot verify DA")]
    NoChunks,
    #[error("last chunk transition decode failed: {0:?}")]
    LastChunkDecode(ssz::DecodeError),
    #[error("DA blob reassembly failed: {0}")]
    Reassembly(CodecError),
    #[error("malformed DA tx in witness: {0}")]
    DaTxDecode(String),
    #[error("DA witness block has no DA transactions")]
    MissingDaTransactions,
    #[error("DA witness block ref is not present in public ledger refs: idx={idx}")]
    L1DaBlockRefNotInLedgerRefs { idx: u64 },
    #[error("DA witness wtxid Merkle root mismatch: expected={expected:?}, computed={computed:?}")]
    WtxidsRootMismatch {
        expected: [u8; 32],
        computed: [u8; 32],
    },
    /// Errors from commit/reveal extraction shared with the host builder.
    #[error("DA parse failure: {0}")]
    Parse(#[from] DaParseError),
    #[error("DA commit OP_RETURN magic mismatch: expected {expected:?}, got {actual:?}")]
    CommitMagicMismatch { expected: [u8; 4], actual: [u8; 4] },
    #[error("DA commit OP_RETURN version mismatch: expected {expected}, got {actual}")]
    CommitVersionMismatch { expected: u32, actual: u32 },
    #[error("DA blocks present but raw_partial_pre_state is empty")]
    MissingPartialPreState,
    #[error("partial pre-state decode failed: {0}")]
    PartialPreStateDecode(CodecError),
    #[error("partial pre-state root mismatch: expected={expected:?}, actual={actual:?}")]
    PartialPreStateRootMismatch {
        expected: [u8; 32],
        actual: [u8; 32],
    },
    #[error("state-diff apply failed: {0}")]
    StateDiffApply(#[from] ReconstructError),
    #[error(
        "post-apply state root does not match last chunk's tip_state_root: computed={computed:?}, expected={expected:?}"
    )]
    StateRootMismatch {
        computed: [u8; 32],
        expected: [u8; 32],
    },
    #[error("DA blob update_seq_no mismatch: expected={expected}, actual={actual}")]
    UpdateSeqNoMismatch { expected: u64, actual: u64 },
    #[error("chunk tip EVM header summary decode failed: {0}")]
    ExecHeaderSummaryDecode(CodecError),
    #[error("DA blob EVM header summary mismatch: expected={expected:?}, actual={actual:?}")]
    EvmHeaderMismatch {
        expected: EvmHeaderSummary,
        actual: EvmHeaderSummary,
    },
    #[error("DA blob missing deployed bytecode for code hash {0:?}")]
    MissingDeployedBytecode([u8; 32]),
    #[error(
        "DA blob deployed bytecode hash mismatch: expected={expected:?}, computed={computed:?}"
    )]
    DeployedBytecodeHashMismatch {
        expected: [u8; 32],
        computed: [u8; 32],
    },
    #[error(
        "DA witness known bytecode hash mismatch: expected={expected:?}, computed={computed:?}"
    )]
    KnownBytecodeHashMismatch {
        expected: [u8; 32],
        computed: [u8; 32],
    },
}
