//! DA codec types and format constants shared between producer and verifier.

use alpen_reth_statediff::BatchStateDiff;
use strata_codec::{Codec, CodecError, Decoder};

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
/// `chunks` must be in commit-output order. The blob is decoded directly across
/// the chunk slices (no intermediate contiguous copy), and any trailing bytes
/// after a complete `DaBlob` are rejected — matching `decode_buf_exact`.
pub fn reassemble_da_blob(chunks: &[Vec<u8>]) -> Result<DaBlob, CodecError> {
    if chunks.is_empty() {
        return Err(CodecError::MalformedField("no DA chunks provided"));
    }

    let mut dec = MultiSliceDecoder::new(chunks);
    let blob = DaBlob::decode(&mut dec)?;
    if dec.remaining() > 0 {
        return Err(CodecError::ExtraInput);
    }
    Ok(blob)
}

/// A [`Decoder`] that reads across a sequence of byte chunks without first
/// concatenating them into one contiguous buffer.
///
/// Lets [`reassemble_da_blob`] decode a [`DaBlob`] straight from its
/// commit/reveal chunk payloads, avoiding an O(blob) allocation + copy on the
/// proof-verification path.
struct MultiSliceDecoder<'a> {
    chunks: &'a [Vec<u8>],
    /// Index of the chunk currently being read.
    chunk: usize,
    /// Read offset within `chunks[chunk]`.
    offset: usize,
}

impl<'a> MultiSliceDecoder<'a> {
    fn new(chunks: &'a [Vec<u8>]) -> Self {
        Self {
            chunks,
            chunk: 0,
            offset: 0,
        }
    }

    /// Total number of unread bytes across the current and later chunks.
    fn remaining(&self) -> usize {
        if self.chunk >= self.chunks.len() {
            return 0;
        }
        let current = self.chunks[self.chunk].len().saturating_sub(self.offset);
        let later: usize = self.chunks[self.chunk + 1..].iter().map(Vec::len).sum();
        current + later
    }
}

impl Decoder for MultiSliceDecoder<'_> {
    fn read_buf(&mut self, into: &mut [u8]) -> Result<(), CodecError> {
        if into.len() > self.remaining() {
            return Err(CodecError::OverrunInput);
        }

        let mut filled = 0;
        while filled < into.len() {
            let chunk = &self.chunks[self.chunk];
            if self.offset >= chunk.len() {
                // Current chunk exhausted; `remaining()` guarantees a later one holds the rest.
                self.chunk += 1;
                self.offset = 0;
                continue;
            }
            let available = &chunk[self.offset..];
            let take = available.len().min(into.len() - filled);
            into[filled..filled + take].copy_from_slice(&available[..take]);
            self.offset += take;
            filled += take;
        }

        Ok(())
    }

    fn read_arr<const N: usize>(&mut self) -> Result<[u8; N], CodecError> {
        let mut buf = [0u8; N];
        self.read_buf(&mut buf)?;
        Ok(buf)
    }
}

#[cfg(test)]
mod tests {
    use strata_codec::encode_to_vec;

    use super::*;

    fn sample_blob() -> DaBlob {
        DaBlob {
            update_seq_no: 7,
            evm_header: EvmHeaderSummary {
                block_num: 10,
                timestamp: 1_700_000_000,
                base_fee: 100,
                gas_used: 21_000,
                gas_limit: 30_000_000,
            },
            state_diff: BatchStateDiff::new(),
        }
    }

    #[test]
    fn reassembles_across_arbitrary_chunk_boundaries() {
        let encoded = encode_to_vec(&sample_blob()).unwrap();

        // Splitting the same bytes at every boundary must decode identically to
        // the single-buffer path (compared via re-encoding, as DaBlob is not Eq).
        for chunk_size in 1..=encoded.len() {
            let chunks: Vec<Vec<u8>> = encoded.chunks(chunk_size).map(|c| c.to_vec()).collect();
            let got = reassemble_da_blob(&chunks).expect("decode across chunks");
            assert_eq!(
                encode_to_vec(&got).unwrap(),
                encoded,
                "chunk_size={chunk_size}"
            );
        }
    }

    #[test]
    fn empty_chunks_is_error() {
        assert!(reassemble_da_blob(&[]).is_err());
    }

    #[test]
    fn trailing_bytes_are_rejected() {
        let mut encoded = encode_to_vec(&sample_blob()).unwrap();
        encoded.push(0xFF);
        assert!(matches!(
            reassemble_da_blob(&[encoded]),
            Err(CodecError::ExtraInput)
        ));
    }
}
