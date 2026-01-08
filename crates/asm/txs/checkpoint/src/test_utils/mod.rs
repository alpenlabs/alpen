mod mmr;

use k256::schnorr::{Signature, SigningKey, signature::Signer};
pub use mmr::{TestMmr, verified_aux_data_for_heights};
use ssz::Encode;
use strata_btc_types::payload::L1Payload;
use strata_checkpoint_types_ssz::{
    BatchInfo, CheckpointCommitment, CheckpointPayload, CheckpointSidecar, L1BlockRange,
    L2BlockRange, OLLog, SignedCheckpointPayload,
};
use strata_identifiers::{
    Buf32, Buf64, Epoch, L1BlockCommitment, L1BlockId, OLBlockCommitment, OLBlockId,
};
use strata_l1_txfmt::TagData;
use strata_predicate::{PredicateKey, PredicateTypeId};
use strata_test_utils::ArbitraryGenerator;

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
        let mut arb = ArbitraryGenerator::new();
        let secret_key_bytes: [u8; 32] = arb.generate();

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
        let signature: Signature = signing_key.sign(&payload_bytes);
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
///
/// Uses `[start, end]` semantics for L1 and L2 ranges where:
/// - `start` is the first block covered by this checkpoint (previous end + 1)
/// - `end` is the last block covered by this checkpoint
#[derive(Debug)]
pub struct CheckpointGenerator {
    epoch: Epoch,
    /// The L1 commitment at which the previous checkpoint ended (or genesis for first checkpoint).
    /// The next checkpoint's L1 range will start at height `last_l1_end.height + 1`.
    last_l1_end: L1BlockCommitment,
    /// The L2 terminal from the previous checkpoint (None before first checkpoint).
    /// The next checkpoint's L2 range will start at slot `last_l2_terminal.slot() + 1` (or 1 for
    /// first).
    last_l2_terminal: Option<OLBlockCommitment>,
    pre_state_root: Buf32,
    arb: ArbitraryGenerator,
}

impl CheckpointGenerator {
    /// Create a new generator seeded with the genesis L1 commitment.
    ///
    /// The first checkpoint's L1 range will start at `genesis_l1.height + 1`.
    pub fn new(genesis_l1: L1BlockCommitment) -> Self {
        Self {
            epoch: 0,
            last_l1_end: genesis_l1,
            last_l2_terminal: None,
            pre_state_root: Buf32::zero(),
            arb: ArbitraryGenerator::new(),
        }
    }

    /// Generate an unsigned checkpoint payload for the next epoch.
    ///
    /// * `l1_blocks` - Number of L1 blocks covered by this checkpoint (must be > 0)
    /// * `l2_slots` - Number of L2 slots covered by this checkpoint (must be > 0)
    /// * `ol_logs` - Optional OL logs to embed in the sidecar
    ///
    /// The L1 range uses `[start, end]` semantics:
    /// - `start.height = last_l1_end.height + 1` (first block covered)
    /// - `end.height = start.height + l1_blocks - 1` (last block covered)
    pub fn gen_payload(
        &mut self,
        l1_blocks: u32,
        l2_slots: u64,
        ol_logs: Vec<OLLog>,
    ) -> CheckpointPayload {
        assert!(l1_blocks > 0, "l1_blocks must be greater than zero");
        assert!(l2_slots > 0, "l2_slots must be greater than zero");

        // L1 range: [start, end] where start = previous_end + 1
        let start_height = self.last_l1_end.height_u64() as u32 + 1;
        let l1_start = L1BlockCommitment::from_height_u64(
            start_height as u64,
            self.arb.generate::<L1BlockId>(),
        )
        .expect("valid L1 height");
        let l1_end = L1BlockCommitment::from_height_u64(
            (start_height + l1_blocks - 1) as u64,
            self.arb.generate::<L1BlockId>(),
        )
        .expect("valid L1 height");

        // L2 range: start = first covered slot (previous_terminal + 1, or 1 for first checkpoint)
        let l2_start_slot = self.last_l2_terminal.map(|t| t.slot() + 1).unwrap_or(1);
        let l2_start = OLBlockCommitment::new(l2_start_slot, self.arb.generate::<OLBlockId>());
        let l2_end = OLBlockCommitment::new(
            l2_start_slot + l2_slots - 1,
            self.arb.generate::<OLBlockId>(),
        );

        let batch_info = BatchInfo::new(
            self.epoch,
            L1BlockRange::new(l1_start, l1_end),
            L2BlockRange::new(l2_start, l2_end),
        );

        let post_state_root = self.arb.generate::<Buf32>();
        let sidecar = CheckpointSidecar::new(Vec::new(), encode_ol_logs(&ol_logs))
            .expect("failed to build sidecar");

        CheckpointPayload::new(
            CheckpointCommitment::new(batch_info, post_state_root),
            sidecar,
            Vec::new(), // empty proof; use PredicateKey::always_accept in tests
        )
        .expect("failed to construct checkpoint payload")
    }

    /// Generate a signed checkpoint payload for the next epoch.
    pub fn gen_signed_payload(
        &mut self,
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
        self.last_l1_end = batch_info.l1_range.end;
        self.last_l2_terminal = Some(batch_info.l2_range.end);
        self.pre_state_root = payload.commitment.post_state_root;
        self.epoch += 1;
    }
}

/// Build an L1 payload containing the signed checkpoint with the proper SPS-50 tag.
pub fn build_l1_payload(signed_checkpoint: &SignedCheckpointPayload) -> L1Payload {
    let tag = TagData::new(CHECKPOINT_SUBPROTOCOL_ID, OL_STF_CHECKPOINT_TX_TYPE, vec![]).unwrap();
    let payload_bytes = signed_checkpoint.as_ssz_bytes();
    L1Payload::new(vec![payload_bytes], tag)
}

/// Clone an `L1BlockCommitment` (convenience wrapper for tests).
pub fn checkpoint_l1_from_block(commitment: &L1BlockCommitment) -> L1BlockCommitment {
    *commitment
}

fn encode_ol_logs(logs: &[OLLog]) -> Vec<u8> {
    if logs.is_empty() {
        return Vec::new();
    }
    logs.to_vec().as_ssz_bytes()
}
