//! Consensus types that track node behavior as we receive messages from the L1
//! chain and the p2p network. These will be expanded further as we actually
//! implement the consensus logic.
// TODO move this to another crate that contains our sync logic

use core::fmt;

use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_primitives::{
    batch::BatchTransition, buf::Buf32, epoch::EpochCommitment, l1::L1BlockCommitment,
};

use crate::{batch::BatchInfo, l1::L1BlockId};

/// High level client's checkpoint view of the network. This is local to the client, not
/// coordinated as part of the L2 chain.
///
/// This is updated when we see a consensus-relevant message.  This is L2 blocks
/// but also L1 blocks being published with relevant things in them, and
/// various other events.
#[derive(
    Clone,
    Debug,
    Default,
    Eq,
    PartialEq,
    Arbitrary,
    BorshSerialize,
    BorshDeserialize,
    Deserialize,
    Serialize,
)]
pub struct ClientState {
    // Last *finalized* checkpoint.
    pub(crate) last_finalized_checkpoint: Option<L1Checkpoint>,

    // Last *seen* checkpoint.
    pub(crate) last_seen_checkpoint: Option<L1Checkpoint>,
}

impl ClientState {
    pub fn new(
        last_finalized_checkpoint: Option<L1Checkpoint>,
        last_seen_checkpoint: Option<L1Checkpoint>,
    ) -> Self {
        ClientState {
            last_finalized_checkpoint,
            last_seen_checkpoint,
        }
    }

    /// Gets the last checkpoint as of the last internal state.
    ///
    /// This isn't durable, as it's possible it might be rolled back in the
    /// future, although it becomes less likely the longer it's buried.
    pub fn get_last_checkpoint(&self) -> Option<L1Checkpoint> {
        self.last_seen_checkpoint.clone()
    }

    /// Gets the last checkpoint that has already been finalized.
    pub fn get_last_finalized_checkpoint(&self) -> Option<L1Checkpoint> {
        self.last_finalized_checkpoint.clone()
    }

    /// Gets the final epoch that we've externally declared.
    pub fn get_declared_final_epoch(&self) -> Option<EpochCommitment> {
        self.last_finalized_checkpoint
            .as_ref()
            .map(|ckpt| ckpt.batch_info.get_epoch_commitment())
    }

    /// Gets the next epoch we expect to be confirmed.
    pub fn get_next_expected_epoch_conf(&self) -> u64 {
        self.last_seen_checkpoint
            .as_ref()
            .map(|ck| ck.batch_info.get_epoch_commitment().epoch() + 1)
            .unwrap_or(0u64)
    }
}

/// A [`ClientState`] wrapper used in StatusChannel.
/// Supplied with block to wait for genesis.
/// TODO: to be reworked.
#[derive(Debug, Clone, Default)]
pub struct CheckpointState {
    pub client_state: ClientState,
    pub block: L1BlockCommitment,
}

impl CheckpointState {
    pub fn new(client_state: ClientState, block: L1BlockCommitment) -> Self {
        Self {
            client_state,
            block,
        }
    }

    pub fn has_genesis_occurred(&self) -> bool {
        self.block.height() > 0
    }
}

/// Represents a reference to a transaction in bitcoin. Redundantly puts block_height a well.
#[derive(
    Clone, Eq, PartialEq, Arbitrary, BorshDeserialize, BorshSerialize, Deserialize, Serialize,
)]
pub struct CheckpointL1Ref {
    pub l1_commitment: L1BlockCommitment,
    pub txid: Buf32,
    pub wtxid: Buf32,
}

impl CheckpointL1Ref {
    pub fn new(l1_commitment: L1BlockCommitment, txid: Buf32, wtxid: Buf32) -> Self {
        Self {
            l1_commitment,
            txid,
            wtxid,
        }
    }

    pub fn block_height(&self) -> u64 {
        self.l1_commitment.height()
    }

    pub fn block_id(&self) -> &L1BlockId {
        self.l1_commitment.blkid()
    }
}

#[derive(
    Clone, Debug, Eq, PartialEq, Arbitrary, BorshDeserialize, BorshSerialize, Deserialize, Serialize,
)]
pub struct L1Checkpoint {
    /// The inner checkpoint batch info.
    pub batch_info: BatchInfo,

    /// The inner checkpoint batch transition.
    pub batch_transition: BatchTransition,

    /// L1 reference for this checkpoint.
    pub l1_reference: CheckpointL1Ref,
}

impl fmt::Display for L1Checkpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        <Self as fmt::Debug>::fmt(self, f)
    }
}

impl L1Checkpoint {
    pub fn new(
        batch_info: BatchInfo,
        batch_transition: BatchTransition,
        l1_reference: CheckpointL1Ref,
    ) -> Self {
        Self {
            batch_info,
            batch_transition,
            l1_reference,
        }
    }
}

// Custom debug implementation to print txid and wtxid in little endian
impl fmt::Debug for CheckpointL1Ref {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let txid_le = {
            let mut bytes = self.txid.0;
            bytes.reverse();
            bytes
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect::<String>()
        };
        let wtxid_le = {
            let mut bytes = self.wtxid.0;
            bytes.reverse();
            bytes
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect::<String>()
        };

        f.debug_struct("CheckpointL1Ref")
            .field("l1_commitment", &self.l1_commitment)
            .field("txid", &txid_le)
            .field("wtxid", &wtxid_le)
            .finish()
    }
}

// Custom display implementation to print txid and wtxid in little endian
impl fmt::Display for CheckpointL1Ref {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let txid_le = {
            let mut bytes = self.txid.0;
            bytes.reverse();
            bytes
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect::<String>()
        };
        let wtxid_le = {
            let mut bytes = self.wtxid.0;
            bytes.reverse();
            bytes
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect::<String>()
        };

        write!(
            f,
            "CheckpointL1Ref {{ l1_commitment: {}, txid: {}, wtxid: {} }}",
            self.l1_commitment, txid_le, wtxid_le
        )
    }
}
