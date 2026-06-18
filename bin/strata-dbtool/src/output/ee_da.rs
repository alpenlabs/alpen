//! Output structures for EE DA inspection results.

use serde::Serialize;

use crate::output::traits::Formattable;

/// Complete output payload for `ee-da-inspect`.
#[derive(Debug, Serialize)]
pub(crate) struct EeDaInspectInfo {
    /// Metadata and local bytes for the selected target DA blob.
    pub(crate) target: EeDaTargetInfo,
    /// Replay result produced from the contiguous DA prefix.
    pub(crate) replay: EeDaReplayInfo,
}

/// Output fields describing the DA blob that covers the requested block.
#[derive(Debug, Serialize)]
pub(crate) struct EeDaTargetInfo {
    /// Index of the chunked-envelope record in the EE sled store.
    pub(crate) envelope_idx: u64,
    /// Monotonic DA update sequence number encoded in the blob.
    pub(crate) update_seq_no: u64,
    /// Last EVM block covered by the selected DA blob.
    pub(crate) last_block_num: u64,
    /// Hex encoding of the producer-local blob bytes.
    pub(crate) local_blob_hex: String,
    /// SHA-256 digest of the producer-local blob bytes, hex encoded.
    pub(crate) local_blob_sha256: String,
    /// Number of stored chunks that formed the local blob bytes.
    pub(crate) chunk_count: usize,
}

/// Output fields describing the result of replaying DA state diffs.
#[derive(Debug, Serialize)]
pub(crate) struct EeDaReplayInfo {
    /// EVM post-state root after replaying the canonical DA prefix.
    pub(crate) post_state_root: String,
}

impl Formattable for EeDaInspectInfo {
    fn format_porcelain(&self) -> String {
        [
            format!("target.envelope_idx: {}", self.target.envelope_idx),
            format!("target.update_seq_no: {}", self.target.update_seq_no),
            format!("target.last_block_num: {}", self.target.last_block_num),
            format!("target.local_blob_hex: {}", self.target.local_blob_hex),
            format!(
                "target.local_blob_sha256: {}",
                self.target.local_blob_sha256
            ),
            format!("target.chunk_count: {}", self.target.chunk_count),
            format!("replay.post_state_root: {}", self.replay.post_state_root),
        ]
        .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_inspection_result_as_porcelain() {
        let info = EeDaInspectInfo {
            target: EeDaTargetInfo {
                envelope_idx: 7,
                update_seq_no: 3,
                last_block_num: 42,
                local_blob_hex: "deadbeef".to_string(),
                local_blob_sha256: "abc123".to_string(),
                chunk_count: 2,
            },
            replay: EeDaReplayInfo {
                post_state_root: "0xfeed".to_string(),
            },
        };

        assert_eq!(
            info.format_porcelain(),
            [
                "target.envelope_idx: 7",
                "target.update_seq_no: 3",
                "target.last_block_num: 42",
                "target.local_blob_hex: deadbeef",
                "target.local_blob_sha256: abc123",
                "target.chunk_count: 2",
                "replay.post_state_root: 0xfeed",
            ]
            .join("\n")
        );
    }
}
