//! DA codec types and format constants shared between producer and verifier.

use alpen_reth_statediff::BatchStateDiff;
use strata_codec::{decode_buf_exact, Codec, CodecError};

/// Magic bytes in the EE DA commit transaction marker output.
///
/// TODO(STR-1907): derive this from authenticated EE proof context instead of
/// baking the network value into runtime/proof code.
pub const EE_DA_MAGIC_BYTES: [u8; 4] = *b"ALPN";

/// Current EE DA blob encoding version.
///
/// The commit transaction carries this version next to the EE DA magic bytes
/// in OP_RETURN, so L1 scanners can associate reassembled blob bytes with the
/// schema that produced them. The current decoder handles only the present
/// [`DaBlob`] shape; version dispatch can be added when a future blob schema
/// is introduced.
///
/// TODO(STR-1907): make this part of the same authenticated EE proof context
/// as chain ID and DA magic bytes.
pub const DA_BLOB_VERSION: u32 = 0;

/// DA blob containing batch metadata and state diff.
///
/// This is the top-level structure that gets encoded and posted to L1. It
/// wraps the batch state diff with sequencing metadata needed for L1 sync and
/// chain reconstruction.
#[derive(Debug, Clone, Codec)]
pub struct DaBlob {
    /// Monotonic EE account update sequence number for this blob.
    pub update_seq_no: u64,
    /// EVM header context of the last block in this batch.
    pub evm_header: EvmHeaderSummary,
    /// Aggregated state diff for the batch (can be empty for batches with no
    /// state changes).
    pub state_diff: BatchStateDiff,
}

/// Compact summary of the last EVM block header in a batch.
///
/// A sequencer rebuilding from L1 DA has the [`BatchStateDiff`] for state
/// changes but not the block headers, so these non-derivable fields let it
/// build the next block: `base_fee`/`gas_used`/`gas_limit` drive the EIP-1559
/// base-fee and gas-limit update, `timestamp` enforces monotonicity, and
/// `block_num` marks where the chain continues.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Codec)]
pub struct EvmHeaderSummary {
    /// Block number of the last EVM block in this batch.
    pub block_num: u64,
    /// Unix timestamp (seconds) of the last EVM block.
    pub timestamp: u64,
    /// Base fee per gas (EIP-1559) of the last EVM block.
    pub base_fee: u64,
    /// Total gas consumed by the last EVM block.
    pub gas_used: u64,
    /// Gas limit of the last EVM block.
    pub gas_limit: u64,
}

/// Reassembles a [`DaBlob`] from raw chunk payloads.
///
/// `chunks` must be in commit-output order.
pub fn reassemble_da_blob(chunks: &[Vec<u8>]) -> Result<DaBlob, CodecError> {
    if chunks.is_empty() {
        return Err(CodecError::MalformedField("no DA chunks provided"));
    }

    let blob: Vec<u8> = chunks.iter().flat_map(|c| c.iter().copied()).collect();
    decode_buf_exact(&blob)
}
