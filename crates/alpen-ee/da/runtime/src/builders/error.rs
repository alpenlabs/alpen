//! Error raised while building a DA witness on the host.

use alpen_ee_da_types::DaParseError;
use strata_codec::CodecError;

/// Error raised while building a [`DaWitness`](alpen_ee_da_types::DaWitness).
///
/// Spans both layers of the build — L1 inclusion (block fetch, wtxid roots,
/// blob reassembly) and EVM dedup resolution (state-diff/bytecode lookups) —
/// and carries enough categorization for the prover to map onto its own
/// retry semantics: provider/store read failures vs. genuinely missing data
/// vs. structurally inconsistent inputs.
///
/// Message convention matches the verifier's `DaVerificationError`: parameters
/// go in parens, never after a colon.
#[derive(Debug, thiserror::Error)]
pub enum DaWitnessBuildError {
    /// A non-genesis batch referenced no DA blocks.
    #[error("non-genesis batch has no DA refs")]
    EmptyDaRefs,
    /// Reading an L1 block from the Bitcoin backend failed.
    #[error("L1 block fetch failed for {block} ({error})")]
    GetBlock { block: String, error: String },
    /// A referenced L1 block carried no transactions.
    #[error("L1 block {0} has no transactions")]
    BlockHasNoTransactions(String),
    /// A fetched L1 block's wtxids root disagreed with its DA ref.
    #[error("L1 block {block} wtxids_root mismatch (DA ref has {expected}, fetched block has {computed})")]
    WtxidsRootMismatch {
        block: String,
        expected: String,
        computed: String,
    },
    /// A DA transaction named by a DA ref was absent from its L1 block.
    #[error("DA tx {txid}/{wtxid} not found in L1 block {block}")]
    DaTxNotFound {
        txid: String,
        wtxid: String,
        block: String,
    },
    /// Commit/reveal extraction from the witnessed transactions failed.
    #[error("extract DA chunks ({0})")]
    Parse(#[from] DaParseError),
    /// Decoding the reassembled chunk payloads into a `DaBlob` failed.
    #[error("reassemble DA blob ({0})")]
    Reassembly(CodecError),
    /// The state-diff provider failed to read a block's diff.
    #[error("state-diff read failed for block {block} ({error})")]
    StateDiffProvider { block: String, error: String },
    /// A batch block has no state diff available.
    #[error("state diff missing for block {0} while building DA witness")]
    StateDiffMissing(String),
    /// The bytecode store failed to read a code hash.
    #[error("bytecode read failed for {hash} ({error})")]
    BytecodeStore { hash: String, error: String },
    /// A deduped bytecode preimage could not be found in any source.
    #[error("missing deduped bytecode {0} while building DA witness")]
    BytecodeMissing(String),
}
