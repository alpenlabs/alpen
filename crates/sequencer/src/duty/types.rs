//! Sequencer duties.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_primitives::{buf::Buf32, hash::compute_borsh_hash};
use strata_state::{batch::Checkpoint, id::L2BlockId};
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

    /// Duty expires after a specific L2 block is finalized
    BlockIdFinalized(L2BlockId),

    /// Duty expires after a specific checkpoint is finalized on bitcoin
    CheckpointIdxFinalized(u64),
}

/// Unique identifier for a duty.
pub type DutyId = Buf32;

/// Duties the sequencer might carry out.
#[derive(Clone, Debug, BorshSerialize, Serialize, Deserialize)]
#[allow(clippy::large_enum_variant)]
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
            Self::CommitBatch(duty) => Expiry::CheckpointIdxFinalized(duty.0.batch_info().epoch()),
        }
    }

    /// Returns a unique identifier for the duty.
    pub fn generate_id(&self) -> Buf32 {
        match self {
            // We want Batch commitment duty to be unique by the checkpoint idx
            Self::CommitBatch(duty) => compute_borsh_hash(&duty.0.batch_info().epoch()),
            _ => compute_borsh_hash(self),
        }
    }
}

/// Describes information associated with signing a block.
#[derive(Clone, Debug, BorshSerialize, Serialize, Deserialize)]
pub struct BlockSigningDuty {
    /// Slot to sign for.
    slot: u64,

    /// Parent to build on
    parent: L2BlockId,

    /// Target timestamp for block
    target_ts: u64,
}

impl BlockSigningDuty {
    /// Create new block signing duty from components.
    pub fn new_simple(slot: u64, parent: L2BlockId, target_ts: u64) -> Self {
        Self {
            slot,
            parent,
            target_ts,
        }
    }

    /// Returns target slot for block signing duty.
    pub fn target_slot(&self) -> u64 {
        self.slot
    }

    /// Returns parent block id for block signing duty.
    pub fn parent(&self) -> L2BlockId {
        self.parent
    }

    /// Returns target ts for block signing duty.
    pub fn target_ts(&self) -> u64 {
        self.target_ts
    }
}

/// This duty is created whenever a previous batch is found on L1 and verified.
/// When this duty is created, in order to execute the duty, the sequencer looks for corresponding
/// batch proof in the proof db.
#[derive(Clone, Debug, BorshSerialize, Serialize, Deserialize)]
pub struct CheckpointDuty(Checkpoint);

impl CheckpointDuty {
    /// Creates a new `CheckpointDuty` from a [`Checkpoint`].
    pub fn new(batch_checkpoint: Checkpoint) -> Self {
        Self(batch_checkpoint)
    }

    /// Consumes `self`, returning the inner [`Checkpoint`].
    pub fn into_inner(self) -> Checkpoint {
        self.0
    }

    /// Returns a reference to the inner [`Checkpoint`].
    pub fn inner(&self) -> &Checkpoint {
        &self.0
    }
}

/// Describes an identity that might be assigned duties.
#[derive(Clone, Debug, BorshDeserialize, BorshSerialize)]
pub enum Identity {
    /// Sequencer with an identity key.
    Sequencer(Buf32),
}

/// Sequencer key used for signing-related duties.
#[derive(Clone, Debug, BorshDeserialize, BorshSerialize, Zeroize, ZeroizeOnDrop)]
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
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    use bitcoin::{bip32::Xpriv, Network};
    use strata_key_derivation::sequencer::SequencerKeys;
    use strata_primitives::buf::Buf32;
    use zeroize::Zeroize;

    use super::*;

    #[test]
    fn test_zeroize_idata() {
        fn load_seqkeys() -> IdentityData {
            let master = Xpriv::new_master(Network::Regtest, &[2u8; 32]).unwrap();
            let keys = SequencerKeys::new(&master).unwrap();
            let mut seq_sk = Buf32::from(keys.derived_xpriv().private_key.secret_bytes());
            let seq_pk = keys.derived_xpub().to_x_only_pub().serialize();
            let ik = IdentityKey::Sequencer(seq_sk);
            let ident = Identity::Sequencer(Buf32::from(seq_pk));
            // Zeroize the Buf32 representation of the Xpriv.
            seq_sk.zeroize();

            IdentityData::new(ident, ik)
        }

        // Create an atomic flag to track if zeroize was called
        let was_zeroized = Arc::new(AtomicBool::new(false));
        let was_zeroized_clone = Arc::clone(&was_zeroized);

        let idata = load_seqkeys();

        // Create a wrapper struct that will set a flag when dropped
        struct TestWrapper {
            inner: IdentityData,
            flag: Arc<AtomicBool>,
        }

        impl Drop for TestWrapper {
            fn drop(&mut self) {
                // Get the current value before the inner value is dropped
                // Extract the Buf32 from the enum
                let bytes = match &self.inner.key {
                    IdentityKey::Sequencer(buf) => buf.as_bytes(),
                };

                // The inner IdentityData will be dropped after this,
                // triggering zeroization
                self.flag.store(bytes != [0u8; 32], Ordering::Relaxed);
            }
        }

        // Create and drop our test wrapper
        {
            let _ = TestWrapper {
                inner: idata,
                flag: was_zeroized_clone,
            };
        }

        // Check if zeroization occurred
        assert!(was_zeroized.load(Ordering::Relaxed));
    }
}
