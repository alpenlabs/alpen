//! Core SPS-62 checkpoint type definitions.

use ssz_derive::{Decode, Encode};
use ssz_types::VariableList;
use tree_hash_derive::TreeHash;

/// Maximum size of OL DA state diff: 256 KiB (2^18 bytes)
pub const OL_DA_DIFF_MAX_SIZE: usize = 1 << 18;

/// Maximum size of output message blob: 16 KiB (2^14 bytes)
pub const OUTPUT_MSG_MAX_SIZE: usize = 1 << 14;

/// A 32-byte buffer (used for block IDs, state roots, public keys, etc.)
pub type Bytes32 = [u8; 32];

/// A 64-byte buffer (used for signatures)
pub type Bytes64 = [u8; 64];

/// Variable-length byte list for OL state diff (max 256 KiB)
pub type OlStateDiff = VariableList<u8, 262144>; // 2^18

/// Variable-length byte list for output messages (max 16 KiB)
pub type OutputMsgBlob = VariableList<u8, 16384>; // 2^14

/// Checkpoint header containing minimally necessary information to construct an EpochSummary.
///
/// This structure is ordered for optimal SSZ packing.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode, TreeHash)]
pub struct CheckpointHeader {
    /// Epoch number
    pub epoch: u32,

    /// L1 view update height (ordering for compactness)
    pub l1_view_update_height: u32,

    /// Terminal slot number
    pub terminal_slot: u64,

    /// Terminal block ID
    pub terminal_blkid: Bytes32,

    /// Final state root
    pub final_state_root: Bytes32,
}

/// Checkpoint payload containing various "output" components.
///
/// This includes all messages that originate from L2 such as withdrawals.
/// This is partly intended to be consumed by ASM and treated as an opaque type.
///
/// Note: The spec shows this as a StableContainer, but the SSZ library doesn't support
/// StableContainer yet, so we use a regular Container with Option fields.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode, TreeHash)]
pub struct CheckpointPayload {
    /// Encoded DA state diff. Checked by proof to ensure it matches state.
    ///
    /// Maximum size: [`OL_DA_DIFF_MAX_SIZE`] (256 KiB)
    pub ol_state_diff: Option<OlStateDiff>,

    /// Output messages from OL. This corresponds to (some subset of?) OL logs.
    ///
    /// Maximum size: [`OUTPUT_MSG_MAX_SIZE`] (16 KiB)
    pub packed_output_msgs: Option<OutputMsgBlob>,
}

/// Container for data we interpret about the checkpoint.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode, TreeHash)]
pub struct CheckpointData {
    /// Checkpoint header
    pub header: CheckpointHeader,

    /// Checkpoint payload
    pub payload: CheckpointPayload,

    /// OL state diff (encoded)
    ///
    /// Maximum size: [`OL_DA_DIFF_MAX_SIZE`] (256 KiB)
    pub ol_state_diff: OlStateDiff,
}

/// Toplevel bundle for the checkpoint data and a proof of it.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode, TreeHash)]
pub struct Checkpoint {
    /// Checkpoint data
    pub data: CheckpointData,

    /// Proof witness (variable-length)
    ///
    /// TODO: Replace with actual unipred::Witness type when available.
    /// For now, using a bounded list. Adjust max size as needed.
    pub proof: VariableList<u8, 262144>, // Max 256 KiB for proof
}

/// Signed checkpoint bundle (does not normally exist on-chain).
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode, TreeHash)]
pub struct SignedCheckpoint {
    /// Schnorr signature (64 bytes)
    pub sig: Bytes64,

    /// Checkpoint bundle
    pub data: Checkpoint,
}

/// Summary of information committing to the final state of an epoch.
///
/// Ordered to pack well in SSZ.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode, TreeHash)]
pub struct EpochSummary {
    /// Epoch index
    pub epoch_idx: u32,

    /// Terminal slot number
    pub terminal_slot: u64,

    /// Last L1 height observed
    pub last_l1_height: u64,

    /// Terminal block ID
    pub terminal_blkid: Bytes32,

    /// Last L1 block ID observed
    pub last_l1_blkid: Bytes32,

    /// Final state root
    pub final_state_root: Bytes32,
}

#[cfg(test)]
mod tests {
    use ssz::{Decode, Encode};

    use super::*;

    #[test]
    fn test_checkpoint_header_ssz_roundtrip() {
        let header = CheckpointHeader {
            epoch: 42,
            l1_view_update_height: 1000,
            terminal_slot: 5000,
            terminal_blkid: [1u8; 32],
            final_state_root: [2u8; 32],
        };

        let encoded = header.as_ssz_bytes();
        let decoded = CheckpointHeader::from_ssz_bytes(&encoded).unwrap();

        assert_eq!(header, decoded);
    }

    #[test]
    fn test_epoch_summary_ssz_roundtrip() {
        let summary = EpochSummary {
            epoch_idx: 10,
            terminal_slot: 1000,
            last_l1_height: 500,
            terminal_blkid: [3u8; 32],
            last_l1_blkid: [4u8; 32],
            final_state_root: [5u8; 32],
        };

        let encoded = summary.as_ssz_bytes();
        let decoded = EpochSummary::from_ssz_bytes(&encoded).unwrap();

        assert_eq!(summary, decoded);
    }

    #[test]
    fn test_checkpoint_payload_with_optional_fields() {
        let payload = CheckpointPayload {
            ol_state_diff: Some(VariableList::from(vec![1, 2, 3, 4])),
            packed_output_msgs: None,
        };

        let encoded = payload.as_ssz_bytes();
        let decoded = CheckpointPayload::from_ssz_bytes(&encoded).unwrap();

        assert_eq!(payload, decoded);
    }

    #[test]
    fn test_checkpoint_data_ssz_roundtrip() {
        let data = CheckpointData {
            header: CheckpointHeader {
                epoch: 1,
                l1_view_update_height: 100,
                terminal_slot: 200,
                terminal_blkid: [6u8; 32],
                final_state_root: [7u8; 32],
            },
            payload: CheckpointPayload {
                ol_state_diff: Some(VariableList::from(vec![10, 20, 30])),
                packed_output_msgs: Some(VariableList::from(vec![40, 50])),
            },
            ol_state_diff: VariableList::from(vec![10, 20, 30]),
        };

        let encoded = data.as_ssz_bytes();
        let decoded = CheckpointData::from_ssz_bytes(&encoded).unwrap();

        assert_eq!(data, decoded);
    }

    #[test]
    fn test_checkpoint_ssz_roundtrip() {
        let checkpoint = Checkpoint {
            data: CheckpointData {
                header: CheckpointHeader {
                    epoch: 2,
                    l1_view_update_height: 200,
                    terminal_slot: 400,
                    terminal_blkid: [8u8; 32],
                    final_state_root: [9u8; 32],
                },
                payload: CheckpointPayload {
                    ol_state_diff: None,
                    packed_output_msgs: Some(VariableList::from(vec![60, 70, 80])),
                },
                ol_state_diff: VariableList::from(vec![]),
            },
            proof: VariableList::from(vec![1, 2, 3, 4, 5]),
        };

        let encoded = checkpoint.as_ssz_bytes();
        let decoded = Checkpoint::from_ssz_bytes(&encoded).unwrap();

        assert_eq!(checkpoint, decoded);
    }

    #[test]
    fn test_signed_checkpoint_ssz_roundtrip() {
        let signed = SignedCheckpoint {
            sig: [10u8; 64],
            data: Checkpoint {
                data: CheckpointData {
                    header: CheckpointHeader {
                        epoch: 3,
                        l1_view_update_height: 300,
                        terminal_slot: 600,
                        terminal_blkid: [12u8; 32],
                        final_state_root: [13u8; 32],
                    },
                    payload: CheckpointPayload {
                        ol_state_diff: Some(VariableList::from(vec![90, 100])),
                        packed_output_msgs: None,
                    },
                    ol_state_diff: VariableList::from(vec![90, 100]),
                },
                proof: VariableList::from(vec![6, 7, 8]),
            },
        };

        let encoded = signed.as_ssz_bytes();
        let decoded = SignedCheckpoint::from_ssz_bytes(&encoded).unwrap();

        assert_eq!(signed, decoded);
    }

    #[test]
    fn test_tree_hash() {
        use tree_hash::TreeHash;

        let header = CheckpointHeader {
            epoch: 42,
            l1_view_update_height: 1000,
            terminal_slot: 5000,
            terminal_blkid: [1u8; 32],
            final_state_root: [2u8; 32],
        };

        // Should not panic and should produce a 32-byte hash
        let hash = header.tree_hash_root();
        assert_eq!(hash.0.len(), 32);
    }
}
