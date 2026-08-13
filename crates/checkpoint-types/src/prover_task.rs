//! Task-key wrapper used by the integrated checkpoint prover.
//!
//! Lives in a shared crate so the running node (`bin/strata`) and offline
//! admin tooling (`bin/strata-dbtool`) agree on the on-disk byte format
//! for entries in the [`strata_db_types::prover_task::ProverTaskDatabase`].
//!
//! Wire format is the fixed-width big-endian concatenation
//! `epoch(4) ‖ last_slot(8) ‖ last_blkid(32)`, 44 bytes total. Fixed-width
//! big-endian keeps the encoding deterministic and lexicographically ordered
//! by epoch, which is what the byte-keyed task tree relies on.

use std::fmt;

use strata_identifiers::{Buf32, Epoch, EpochCommitment, OLBlockId, Slot};
use thiserror::Error;

/// Width of the encoded epoch index.
const EPOCH_LEN: usize = size_of::<Epoch>();

/// Width of the encoded terminal slot.
const SLOT_LEN: usize = size_of::<Slot>();

/// Width of the encoded terminal block id.
const BLKID_LEN: usize = 32;

/// Total width of an encoded [`CheckpointProofTask`].
const KEY_LEN: usize = EPOCH_LEN + SLOT_LEN + BLKID_LEN;

/// Error decoding a [`CheckpointProofTask`] from its key bytes.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("malformed checkpoint proof task key (expected {KEY_LEN} bytes, got {0})")]
pub struct CheckpointProofTaskKeyError(usize);

/// Task identifier for an integrated checkpoint proof.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct CheckpointProofTask(pub EpochCommitment);

impl CheckpointProofTask {
    /// Returns the underlying epoch commitment.
    pub fn commitment(&self) -> EpochCommitment {
        self.0
    }

    /// Encodes the task as its database key bytes.
    ///
    /// See the module docs for the layout.
    pub fn to_key_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(KEY_LEN);
        out.extend_from_slice(&self.0.epoch.to_be_bytes());
        out.extend_from_slice(&self.0.last_slot.to_be_bytes());
        out.extend_from_slice(self.0.last_blkid.as_ref());
        out
    }

    /// Decodes a task from its database key bytes.
    pub fn from_key_bytes(bytes: &[u8]) -> Result<Self, CheckpointProofTaskKeyError> {
        if bytes.len() != KEY_LEN {
            return Err(CheckpointProofTaskKeyError(bytes.len()));
        }

        let (epoch_bytes, rest) = bytes.split_at(EPOCH_LEN);
        let (slot_bytes, blkid_bytes) = rest.split_at(SLOT_LEN);

        let epoch = Epoch::from_be_bytes(epoch_bytes.try_into().expect("epoch width checked"));
        let last_slot = Slot::from_be_bytes(slot_bytes.try_into().expect("slot width checked"));
        let blkid: [u8; BLKID_LEN] = blkid_bytes.try_into().expect("blkid width checked");

        Ok(Self(EpochCommitment::new(
            epoch,
            last_slot,
            OLBlockId::from(Buf32::from(blkid)),
        )))
    }
}

impl From<CheckpointProofTask> for Vec<u8> {
    fn from(task: CheckpointProofTask) -> Self {
        task.to_key_bytes()
    }
}

impl TryFrom<Vec<u8>> for CheckpointProofTask {
    type Error = CheckpointProofTaskKeyError;

    fn try_from(bytes: Vec<u8>) -> Result<Self, Self::Error> {
        Self::from_key_bytes(&bytes)
    }
}

impl fmt::Display for CheckpointProofTask {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task() -> CheckpointProofTask {
        CheckpointProofTask(EpochCommitment::new(
            0x0102_0304,
            0x0A0B_0C0D_0E0F_1011,
            OLBlockId::from(Buf32::from([0x77u8; 32])),
        ))
    }

    /// Pins the on-disk key layout; drift would orphan existing task entries.
    #[test]
    fn key_bytes_match_on_disk_layout() {
        #[rustfmt::skip]
        let expected: Vec<u8> = [
            vec![1, 2, 3, 4],                          // epoch (u32 BE)
            vec![0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10, 0x11], // last_slot (u64 BE)
            vec![0x77; 32],                            // last_blkid
        ]
        .concat();

        assert_eq!(task().to_key_bytes(), expected);
        assert_eq!(expected.len(), KEY_LEN);
    }

    #[test]
    fn key_bytes_roundtrip() {
        let encoded: Vec<u8> = task().into();
        assert_eq!(CheckpointProofTask::try_from(encoded), Ok(task()));
    }

    #[test]
    fn key_bytes_reject_wrong_length() {
        assert!(CheckpointProofTask::from_key_bytes(&[0u8; KEY_LEN - 1]).is_err());
        assert!(CheckpointProofTask::from_key_bytes(&[0u8; KEY_LEN + 1]).is_err());
    }

    /// Lexicographic byte order must follow epoch order so the byte-keyed task tree can be
    /// range-scanned by epoch.
    #[test]
    fn key_bytes_sort_by_epoch() {
        let low = CheckpointProofTask(EpochCommitment::new(
            1,
            u64::MAX,
            OLBlockId::from(Buf32::from([0xFFu8; 32])),
        ));
        let high = CheckpointProofTask(EpochCommitment::new(
            2,
            0,
            OLBlockId::from(Buf32::from([0x00u8; 32])),
        ));
        assert!(low.to_key_bytes() < high.to_key_bytes());
    }
}
