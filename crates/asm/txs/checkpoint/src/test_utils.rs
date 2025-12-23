#![cfg(any(test, feature = "test-utils"))]

use k256::schnorr::{SigningKey, signature::Signer};
use rand::{RngCore, rngs::OsRng};
use ssz::Encode;
use strata_btc_types::payload::L1Payload;
use strata_checkpoint_types_ssz::{
    BatchInfo, BatchTransition, CheckpointCommitment, CheckpointPayload, CheckpointSidecar,
    L1BlockRange, L1Commitment, L2BlockRange, OLLog, SignedCheckpointPayload,
};
use strata_identifiers::{Buf32, Buf64, Epoch, L1BlockCommitment, L1BlockId, OLBlockCommitment};
use strata_l1_txfmt::TagData;
use strata_predicate::{PredicateKey, PredicateTypeId};

use crate::{CHECKPOINT_SUBPROTOCOL_ID, OL_STF_CHECKPOINT_TX_TYPE};

/// Sequencer keypair helper for signing checkpoint payloads.
///
/// Uses k256 for signing to be compatible with the predicate framework's
/// Schnorr BIP-340 verifier, which signs/verifies raw message bytes.
#[derive(Clone, Debug)]
pub struct SequencerKeypair {
    secret_key_bytes: [u8; 32],
    public_key: Buf32,
}

impl SequencerKeypair {
    /// Generate a random sequencer keypair.
    pub fn random() -> Self {
        let mut secret_key_bytes = [0u8; 32];
        OsRng.fill_bytes(&mut secret_key_bytes);

        let signing_key = SigningKey::from_bytes(&secret_key_bytes)
            .expect("random bytes should form a valid signing key");
        let pk_bytes: [u8; 32] = signing_key.verifying_key().to_bytes().into();
        let public_key: Buf32 = pk_bytes.into();

        Self {
            secret_key_bytes,
            public_key,
        }
    }

    /// Sign a checkpoint payload with the sequencer key.
    ///
    /// Signs the raw SSZ-encoded payload bytes (not pre-hashed).
    /// This is compatible with the predicate framework's Schnorr BIP-340 verifier.
    pub fn sign(&self, payload: &CheckpointPayload) -> Buf64 {
        let signing_key = SigningKey::from_bytes(&self.secret_key_bytes)
            .expect("secret key bytes should be valid");
        let payload_bytes = payload.as_ssz_bytes();
        let signature: k256::schnorr::Signature = signing_key.sign(&payload_bytes);
        Buf64::from(signature.to_bytes())
    }

    /// Get the public key.
    pub fn public_key(&self) -> Buf32 {
        self.public_key
    }

    /// Get the sequencer predicate for this keypair.
    ///
    /// Returns a `PredicateKey` with type `Bip340Schnorr` containing the public key.
    pub fn sequencer_predicate(&self) -> PredicateKey {
        PredicateKey::new(
            PredicateTypeId::Bip340Schnorr,
            self.public_key.as_ref().to_vec(),
        )
    }
}

/// Stateful generator for checkpoint payloads.
#[derive(Clone, Debug)]
pub struct CheckpointGenerator {
    epoch: Epoch,
    last_l1: L1Commitment,
    last_l2_terminal: Option<OLBlockCommitment>,
    pre_state_root: Buf32,
}

impl CheckpointGenerator {
    /// Create a new generator seeded with the genesis L1 commitment.
    pub fn new(genesis_l1: L1Commitment) -> Self {
        Self {
            epoch: 0,
            last_l1: genesis_l1,
            last_l2_terminal: None,
            pre_state_root: Buf32::zero(),
        }
    }

    /// Generate an unsigned checkpoint payload for the next epoch.
    ///
    /// * `l1_blocks` - Number of L1 blocks covered by this checkpoint (must be > 0)
    /// * `l2_slots` - Number of L2 slots covered by this checkpoint (must be > 0)
    /// * `ol_logs` - Optional OL logs to embed in the sidecar
    pub fn gen_payload(
        &self,
        l1_blocks: u32,
        l2_slots: u64,
        ol_logs: Vec<OLLog>,
    ) -> CheckpointPayload {
        assert!(l1_blocks > 0, "l1_blocks must be greater than zero");
        assert!(l2_slots > 0, "l2_slots must be greater than zero");

        let l1_start = self.last_l1;
        let l1_end = L1Commitment {
            height: l1_start.height + l1_blocks,
            blkid: random_l1_block_id(),
        };

        let l2_start = self
            .last_l2_terminal
            .unwrap_or_else(OLBlockCommitment::null);
        let l2_end = OLBlockCommitment::new(l2_start.slot() + l2_slots, random_ol_block_id());

        let batch_info = BatchInfo::new(
            self.epoch,
            L1BlockRange::new(l1_start, l1_end),
            L2BlockRange::new(l2_start, l2_end),
        );

        let transition = BatchTransition::new(self.pre_state_root, random_buf32());
        let sidecar = CheckpointSidecar::new(Vec::new(), encode_ol_logs(&ol_logs))
            .expect("failed to build sidecar");

        CheckpointPayload::new(
            CheckpointCommitment::new(batch_info, transition),
            sidecar,
            Vec::new(), // empty proof; use PredicateKey::always_accept in tests
        )
        .expect("failed to construct checkpoint payload")
    }

    /// Generate a signed checkpoint payload for the next epoch.
    pub fn gen_signed_payload(
        &self,
        l1_blocks: u32,
        l2_slots: u64,
        ol_logs: Vec<OLLog>,
        keypair: &SequencerKeypair,
    ) -> SignedCheckpointPayload {
        let payload = self.gen_payload(l1_blocks, l2_slots, ol_logs);
        let signature = keypair.sign(&payload);
        SignedCheckpointPayload::new(payload, signature)
    }

    /// Advance the generator state after a payload has been accepted.
    pub fn advance(&mut self, payload: &CheckpointPayload) {
        let batch_info = &payload.commitment.batch_info;
        self.last_l1 = batch_info.l1_range.end;
        self.last_l2_terminal = Some(batch_info.l2_range.end);
        self.pre_state_root = payload.commitment.transition.post_state_root;
        self.epoch += 1;
    }
}

/// Build an L1 payload containing the signed checkpoint with the proper SPS-50 tag.
pub fn build_l1_payload(signed_checkpoint: &SignedCheckpointPayload) -> L1Payload {
    let tag = TagData::new(CHECKPOINT_SUBPROTOCOL_ID, OL_STF_CHECKPOINT_TX_TYPE, vec![]).unwrap();
    let payload_bytes = signed_checkpoint.as_ssz_bytes();
    L1Payload::new(vec![payload_bytes], tag)
}

/// Convert an `L1BlockCommitment` into a checkpoint `L1Commitment`.
pub fn checkpoint_l1_from_block(commitment: &L1BlockCommitment) -> L1Commitment {
    L1Commitment {
        height: commitment.height_u64() as u32,
        blkid: *commitment.blkid(),
    }
}

fn encode_ol_logs(logs: &[OLLog]) -> Vec<u8> {
    if logs.is_empty() {
        return Vec::new();
    }
    logs.to_vec().as_ssz_bytes()
}

fn random_buf32() -> Buf32 {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    bytes.into()
}

fn random_l1_block_id() -> L1BlockId {
    random_buf32().into()
}

fn random_ol_block_id() -> strata_identifiers::OLBlockId {
    random_buf32().into()
}
