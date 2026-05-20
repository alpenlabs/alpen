//! Consensus types that track node behavior as we receive messages from the L1
//! chain and the p2p network. These will be expanded further as we actually
//! implement the consensus logic.

use core::fmt;

use arbitrary::{Arbitrary, Unstructured};
use borsh::{
    io::{Read, Result as BorshIoResult, Write},
    BorshDeserialize, BorshSerialize,
};
use serde::{Deserialize, Serialize};
use strata_asm_proto_checkpoint_types::CheckpointTip;
use strata_identifiers::{
    Epoch, EpochCommitment, L1BlockCommitment, L1BlockId, L1Height, OLBlockCommitment, RBuf32,
};

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

    /// Gets the last epoch seen on L1.
    pub fn get_last_epoch(&self) -> Option<EpochCommitment> {
        self.last_seen_checkpoint
            .as_ref()
            .map(EpochCommitment::from)
    }

    /// Gets the last checkpoint that has already been finalized.
    pub fn get_last_finalized_checkpoint(&self) -> Option<L1Checkpoint> {
        self.last_finalized_checkpoint.clone()
    }

    /// Gets the final epoch that we've externally declared.
    pub fn get_declared_final_epoch(&self) -> Option<EpochCommitment> {
        self.last_finalized_checkpoint
            .as_ref()
            .map(EpochCommitment::from)
    }

    /// Gets the next epoch we expect to be confirmed.
    pub fn get_next_expected_epoch_conf(&self) -> Epoch {
        self.last_seen_checkpoint
            .as_ref()
            .map(|ck| ck.tip.epoch + 1)
            .unwrap_or(0u32)
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
///
/// `txid` and `wtxid` use [`RBuf32`] so their `Debug`/`Display` follow Bitcoin's
/// reversed-byte hash convention.
#[derive(
    Clone, Debug, Eq, PartialEq, Arbitrary, BorshDeserialize, BorshSerialize, Deserialize, Serialize,
)]
pub struct CheckpointL1Ref {
    pub l1_commitment: L1BlockCommitment,
    pub txid: RBuf32,
    pub wtxid: RBuf32,
}

impl CheckpointL1Ref {
    pub fn new(l1_commitment: L1BlockCommitment, txid: RBuf32, wtxid: RBuf32) -> Self {
        Self {
            l1_commitment,
            txid,
            wtxid,
        }
    }

    pub fn block_height(&self) -> L1Height {
        self.l1_commitment.height()
    }

    pub fn block_id(&self) -> &L1BlockId {
        self.l1_commitment.blkid()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Deserialize, Serialize)]
pub struct L1Checkpoint {
    /// Tip published by the ASM checkpoint subprotocol for this checkpoint.
    ///
    /// `tip.l1_height` is the L1 view consumed by the OL for this epoch — distinct
    /// from `l1_reference.l1_commitment`, which records where the checkpoint
    /// envelope was observed on L1.
    #[borsh(
        serialize_with = "borsh_serialize_checkpoint_tip",
        deserialize_with = "borsh_deserialize_checkpoint_tip"
    )]
    pub tip: CheckpointTip,

    /// L1 reference for the envelope that carried this checkpoint.
    pub l1_reference: CheckpointL1Ref,
}

impl fmt::Display for L1Checkpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        <Self as fmt::Debug>::fmt(self, f)
    }
}

impl L1Checkpoint {
    pub fn new(tip: CheckpointTip, l1_reference: CheckpointL1Ref) -> Self {
        Self { tip, l1_reference }
    }
}

impl From<&L1Checkpoint> for EpochCommitment {
    fn from(checkpoint: &L1Checkpoint) -> Self {
        EpochCommitment::from_terminal(checkpoint.tip.epoch, checkpoint.tip.l2_commitment)
    }
}

// `CheckpointTip` lives in an external crate and doesn't derive `Arbitrary`.
// Construct one field-by-field; the orphan rule prevents impl-ing `Arbitrary`
// directly on `CheckpointTip`.
impl<'a> Arbitrary<'a> for L1Checkpoint {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let tip = CheckpointTip {
            epoch: Epoch::arbitrary(u)?,
            l1_height: L1Height::arbitrary(u)?,
            l2_commitment: OLBlockCommitment::arbitrary(u)?,
        };
        let l1_reference = CheckpointL1Ref::arbitrary(u)?;
        Ok(Self { tip, l1_reference })
    }
}

// Field-level Borsh codec for `CheckpointTip`: the external SSZ-generated type
// has no Borsh derive and the orphan rule blocks adding one. Order matches the
// struct definition.
fn borsh_serialize_checkpoint_tip<W: Write>(
    tip: &CheckpointTip,
    writer: &mut W,
) -> BorshIoResult<()> {
    BorshSerialize::serialize(&tip.epoch, writer)?;
    BorshSerialize::serialize(&tip.l1_height, writer)?;
    BorshSerialize::serialize(&tip.l2_commitment, writer)?;
    Ok(())
}

fn borsh_deserialize_checkpoint_tip<R: Read>(reader: &mut R) -> BorshIoResult<CheckpointTip> {
    let epoch = BorshDeserialize::deserialize_reader(reader)?;
    let l1_height = BorshDeserialize::deserialize_reader(reader)?;
    let l2_commitment = BorshDeserialize::deserialize_reader(reader)?;
    Ok(CheckpointTip {
        epoch,
        l1_height,
        l2_commitment,
    })
}

#[cfg(test)]
mod tests {
    use strata_test_utils::ArbitraryGenerator;

    use super::*;

    #[test]
    fn l1_checkpoint_borsh_roundtrip() {
        let original: L1Checkpoint = ArbitraryGenerator::new().generate();
        let bytes = borsh::to_vec(&original).expect("borsh encode");
        let decoded: L1Checkpoint = borsh::from_slice(&bytes).expect("borsh decode");
        assert_eq!(decoded, original);
    }
}
