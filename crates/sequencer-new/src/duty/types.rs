//! Sequencer duties.

use strata_checkpoint_types_ssz::CheckpointPayload;
use strata_crypto::hash::raw;
use strata_identifiers::{Buf32, Epoch, OLBlockId, Slot};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Describes when we'll stop working to fulfill a duty.
#[derive(Clone, Debug)]
pub enum Expiry {
    /// Duty expires when we see the next block.
    NextBlock,

    /// Duty expires when block is finalized to L1 in a batch.
    BlockFinalized,

    /// Duty expires after a certain timestamp.
    Timestamp(u64),

    /// Duty expires after a specific OL block is finalized.
    BlockIdFinalized(OLBlockId),

    /// Duty expires after a specific checkpoint is finalized on bitcoin.
    ///
    /// Epochs are sealed according to the configured sealing policy.
    CheckpointIdxFinalized(Epoch),
}

/// Unique identifier for a duty.
pub type DutyId = Buf32;

/// Duties the sequencer might carry out.
#[derive(Clone, Debug)]
pub enum Duty {
    /// Goal to sign a block.
    SignBlock(BlockSigningDuty),

    /// Goal to build and commit a batch.
    CommitBatch(CheckpointDuty),
}

impl Duty {
    /// Returns when the duty should expire.
    pub fn expiry(&self) -> Expiry {
        match self {
            Self::SignBlock(_) => Expiry::NextBlock,
            Self::CommitBatch(duty) => Expiry::CheckpointIdxFinalized(duty.0.new_tip().epoch),
        }
    }

    /// Returns a unique identifier for the duty.
    pub fn generate_id(&self) -> Buf32 {
        match self {
            // Ensure checkpoint commitment duty is unique by epoch.
            Self::CommitBatch(duty) => raw(&duty.0.new_tip().epoch.to_be_bytes()),
            Self::SignBlock(duty) => {
                let mut buf = [0u8; 8 + 32 + 8];
                buf[..8].copy_from_slice(&duty.slot.to_be_bytes());
                buf[8..40].copy_from_slice(duty.parent.as_ref());
                buf[40..].copy_from_slice(&duty.target_ts.to_be_bytes());
                raw(&buf)
            }
        }
    }
}

/// Describes information associated with signing a block.
#[derive(Clone, Debug)]
pub struct BlockSigningDuty {
    /// Slot to sign for.
    slot: Slot,

    /// Parent to build on.
    parent: OLBlockId,

    /// Target timestamp for block.
    target_ts: u64,
}

impl BlockSigningDuty {
    /// Create new block signing duty from components.
    pub fn new_simple(slot: Slot, parent: OLBlockId, target_ts: u64) -> Self {
        Self {
            slot,
            parent,
            target_ts,
        }
    }

    /// Returns target slot for block signing duty.
    pub fn target_slot(&self) -> Slot {
        self.slot
    }

    /// Returns parent block id for block signing duty.
    pub fn parent(&self) -> OLBlockId {
        self.parent
    }

    /// Returns target ts for block signing duty.
    pub fn target_ts(&self) -> u64 {
        self.target_ts
    }
}

/// This duty is created whenever a previous checkpoint is found on L1 and verified.
/// When this duty is created, in order to execute the duty, the sequencer looks for the
/// corresponding checkpoint proof in the proof db.
#[derive(Clone, Debug)]
pub struct CheckpointDuty(CheckpointPayload);

impl CheckpointDuty {
    /// Creates a new `CheckpointDuty` from a [`CheckpointPayload`].
    pub fn new(batch_checkpoint: CheckpointPayload) -> Self {
        Self(batch_checkpoint)
    }

    /// Consumes `self`, returning the inner [`CheckpointPayload`].
    pub fn into_inner(self) -> CheckpointPayload {
        self.0
    }

    /// Returns a reference to the inner [`CheckpointPayload`].
    pub fn inner(&self) -> &CheckpointPayload {
        &self.0
    }
}

/// Describes an identity that might be assigned duties.
#[derive(Clone, Debug)]
pub enum Identity {
    /// Sequencer with an identity key.
    Sequencer(Buf32),
}

/// Sequencer key used for signing-related duties.
#[derive(Clone, Debug, Zeroize, ZeroizeOnDrop)]
pub enum IdentityKey {
    /// Sequencer private key used for signing.
    Sequencer(Buf32),
}

/// Container for signing identity key and verification identity key.
///
/// This is really just a stub that we should replace
/// with real cryptographic signatures and putting keys in the rollup params.
#[derive(Clone, Debug)]
pub struct IdentityData {
    /// Unique identifying info.
    pub ident: Identity,

    /// Signing key.
    pub key: IdentityKey,
}

impl IdentityData {
    /// Create new IdentityData from components.
    pub fn new(ident: Identity, key: IdentityKey) -> Self {
        Self { ident, key }
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use strata_acct_types::AccountSerial;
    use strata_checkpoint_types_ssz::{CheckpointPayload, CheckpointSidecar, CheckpointTip};
    use strata_identifiers::{Buf32, OLBlockCommitment, OLBlockId};
    use strata_ol_chain_types_new::OLLog;

    use super::*;

    proptest! {
        #[test]
        fn block_signing_duty_accessors(
            slot in any::<u64>(),
            parent_bytes in any::<[u8; 32]>(),
            target_ts in any::<u64>(),
        ) {
            let parent = OLBlockId::from(Buf32::from(parent_bytes));
            let duty = BlockSigningDuty::new_simple(slot, parent, target_ts);
            prop_assert_eq!(duty.target_slot(), slot);
            prop_assert_eq!(duty.parent(), parent);
            prop_assert_eq!(duty.target_ts(), target_ts);
        }

        #[test]
        fn duty_expiry_for_checkpoint_uses_epoch(
            epoch in any::<u32>(),
            l1_height in any::<u32>(),
            commitment_slot in any::<u64>(),
            commitment_bytes in any::<[u8; 32]>(),
            state_diff in prop::collection::vec(any::<u8>(), 0..64),
            log_payload in prop::collection::vec(any::<u8>(), 0..64),
            proof in prop::collection::vec(any::<u8>(), 0..64),
        ) {
            let blkid = OLBlockId::from(Buf32::from(commitment_bytes));
            let commitment = OLBlockCommitment::new(commitment_slot, blkid);
            let tip = CheckpointTip::new(epoch, l1_height, commitment);

            let log = OLLog::new(AccountSerial::from(0u32), log_payload);
            let sidecar = CheckpointSidecar::new(state_diff, vec![log]).unwrap();
            let payload = CheckpointPayload::new(tip, sidecar, proof).unwrap();

            let duty = Duty::CommitBatch(CheckpointDuty::new(payload));
            match duty.expiry() {
                Expiry::CheckpointIdxFinalized(exp_epoch) => prop_assert_eq!(exp_epoch, epoch),
                other => prop_assert!(false, "unexpected expiry: {other:?}"),
            }
        }
    }
}
