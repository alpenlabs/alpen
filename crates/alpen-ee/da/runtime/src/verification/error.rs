//! Error types for EE DA proof verification.

use alpen_ee_da_types::{DaParseError, EvmHeaderSummary};
use alpen_reth_statediff::ReconstructError;
use strata_codec::CodecError;

/// Result type used throughout DA verification. Defaults the success type to
/// `()` for the common "verify and return nothing" case; pass a type parameter
/// for helpers that produce data along the way (e.g. decoded chunk transitions).
pub type DaVerificationResult<T = ()> = Result<T, DaVerificationError>;

/// Errors raised while verifying EE DA witness data.
#[derive(Debug, thiserror::Error)]
pub enum DaVerificationError {
    #[error("batch under proof has chunks but no DA witness blocks")]
    MissingDaWitness,
    #[error("batch under proof has no chunks; cannot verify DA")]
    NoChunks,
    #[error("last chunk transition decode failed ({0:?})")]
    LastChunkDecode(ssz::DecodeError),
    #[error("DA blob reassembly failed ({0})")]
    Reassembly(CodecError),
    #[error("malformed DA tx in witness ({0})")]
    DaTxDecode(String),
    #[error("DA witness block has no DA transactions")]
    MissingDaTransactions,
    #[error("DA witness block ref not present in public ledger refs (idx {idx})")]
    L1DaBlockRefNotInLedgerRefs { idx: u64 },
    /// The block ref *is* committed, but the witnessed transactions don't hash to
    /// its committed wtxids root (bad inclusion proof / tampered witness txs).
    #[error("DA witness wtxid Merkle root mismatch (expected {expected:?}, got {computed:?})")]
    WtxidsRootMismatch {
        expected: [u8; 32],
        computed: [u8; 32],
    },

    /// Errors from commit/reveal extraction shared with the host builder.
    #[error("DA parse failure ({0})")]
    Parse(#[from] DaParseError),
    #[error("DA commit OP_RETURN magic mismatch (expected {expected:?}, got {actual:?})")]
    CommitMagicMismatch { expected: [u8; 4], actual: [u8; 4] },
    #[error("DA commit OP_RETURN version mismatch (expected {expected}, got {actual})")]
    CommitVersionMismatch { expected: u32, actual: u32 },

    // Pre-state witness. "Partial pre-state" is the `EvmPartialState` sparse-MPT
    // witness: only the trie nodes the batch touches, enough to re-apply the diff
    // and recompute the root — not a full state trie.
    #[error("DA blocks present but raw_partial_pre_state is empty")]
    MissingPartialPreState,
    #[error("partial pre-state decode failed ({0})")]
    PartialPreStateDecode(CodecError),

    // The three state-root / apply stages, in order:
    /// 1. The supplied partial pre-state's root doesn't match the EE account's previous execution
    ///    root (wrong witness for this batch).
    #[error("partial pre-state root mismatch (expected {expected:?}, got {actual:?})")]
    PartialPreStateRootMismatch {
        expected: [u8; 32],
        actual: [u8; 32],
    },
    /// 2. Applying the DA blob's state diff to the pre-state witness errored (e.g. a touched node
    ///    was missing from the partial trie).
    #[error("state-diff apply failed ({0})")]
    StateDiffApply(#[from] ReconstructError),
    /// 3. The apply succeeded, but the resulting root doesn't match the last chunk's committed
    ///    `tip_state_root`.
    #[error("post-apply state root does not match last chunk's tip_state_root (expected {expected:?}, got {computed:?})")]
    PostApplyStateRootMismatch {
        computed: [u8; 32],
        expected: [u8; 32],
    },

    #[error("DA blob update_seq_no mismatch (expected {expected}, got {actual})")]
    UpdateSeqNoMismatch { expected: u64, actual: u64 },
    #[error("chunk tip EVM header summary decode failed ({0})")]
    ExecHeaderSummaryDecode(CodecError),
    #[error("DA blob EVM header summary mismatch (expected {expected:?}, got {actual:?})")]
    EvmHeaderMismatch {
        expected: Box<EvmHeaderSummary>,
        actual: Box<EvmHeaderSummary>,
    },
    #[error("DA blob missing deployed bytecode for code hash {0:?}")]
    MissingDeployedBytecode([u8; 32]),
    #[error("DA blob deployed bytecode hash mismatch (expected {expected:?}, got {computed:?})")]
    DeployedBytecodeHashMismatch {
        expected: [u8; 32],
        computed: [u8; 32],
    },
}
