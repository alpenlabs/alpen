//! Test fixtures for checkpoint subprotocol testing.
//!
//! Provides generators for checkpoint-related test data including batch info,
//! transitions, sidecars, and signed payloads.

use bitcoin::{
    absolute::Height,
    secp256k1::{Secp256k1, SecretKey},
};
use rand::{Rng, RngCore};
use strata_checkpoint_types_ssz::{
    BatchInfo, BatchTransition, CheckpointPayload, CheckpointSidecar, L1BlockRange, L2BlockRange,
    SignedCheckpointPayload,
};
use strata_crypto::schnorr::sign_schnorr_sig;
use strata_identifiers::{
    Buf32, Buf64, Epoch, L1BlockCommitment, L1BlockId, L2BlockCommitment, L2BlockId, Slot,
};

/// A sequencer keypair for signing checkpoints.
#[derive(Clone, Debug)]
pub struct SequencerKeypair {
    /// Private key (32 bytes).
    pub secret_key: Buf32,
    /// Public key (x-only, 32 bytes).
    pub public_key: Buf32,
}

impl SequencerKeypair {
    /// Generate a new random sequencer keypair.
    pub fn random() -> Self {
        let secp = Secp256k1::new();
        let mut rng = rand::thread_rng();
        let mut sk_bytes = [0u8; 32];
        rng.fill_bytes(&mut sk_bytes);

        let sk = SecretKey::from_slice(&sk_bytes).expect("valid key");
        let (pk, _parity) = sk.x_only_public_key(&secp);

        Self {
            secret_key: Buf32::from(sk_bytes),
            public_key: Buf32::from(pk.serialize()),
        }
    }

    /// Sign a message hash with this keypair.
    pub fn sign(&self, msg: &Buf32) -> Buf64 {
        sign_schnorr_sig(msg, &self.secret_key)
    }
}

/// Collection of checkpoint test fixtures with consistent state.
#[derive(Clone, Debug)]
pub struct CheckpointFixtures {
    /// Sequencer keypair for signing.
    pub sequencer: SequencerKeypair,
    /// Current epoch being tested.
    pub epoch: Epoch,
    /// Genesis L1 block commitment.
    pub genesis_l1: L1BlockCommitment,
    /// Genesis L2 block commitment.
    pub genesis_l2: L2BlockCommitment,
}

impl Default for CheckpointFixtures {
    fn default() -> Self {
        Self::new()
    }
}

impl CheckpointFixtures {
    /// Create new fixtures with random keys and epoch 0.
    pub fn new() -> Self {
        Self {
            sequencer: SequencerKeypair::random(),
            epoch: 0,
            genesis_l1: gen_l1_block_commitment(100),
            genesis_l2: gen_l2_block_commitment(0),
        }
    }

    /// Create fixtures starting at a specific epoch.
    pub fn with_epoch(epoch: Epoch) -> Self {
        Self {
            epoch,
            ..Self::new()
        }
    }

    /// Generate a checkpoint payload for the current epoch.
    pub fn gen_payload(&self) -> CheckpointPayload {
        self.gen_payload_for_epoch(self.epoch)
    }

    /// Generate a checkpoint payload for a specific epoch.
    pub fn gen_payload_for_epoch(&self, epoch: Epoch) -> CheckpointPayload {
        let l1_start_height = self.genesis_l1.height_u64() + (epoch as u64 * 10);
        let l1_end_height = l1_start_height + 10;

        let l2_start_slot = self.genesis_l2.slot() + (epoch as u64 * 100);
        let l2_end_slot = l2_start_slot + 100;

        let l1_range = L1BlockRange::new(
            gen_l1_block_commitment(l1_start_height),
            gen_l1_block_commitment(l1_end_height),
        );

        let l2_range = L2BlockRange::new(
            gen_l2_block_commitment(l2_start_slot),
            gen_l2_block_commitment(l2_end_slot),
        );

        let batch_info = BatchInfo::new(epoch, l1_range, l2_range);

        let pre_state_root = gen_random_buf32();
        let post_state_root = gen_random_buf32();
        let transition = BatchTransition::new(epoch, pre_state_root, post_state_root);

        let sidecar = gen_checkpoint_sidecar();
        let proof = gen_dummy_proof();

        CheckpointPayload::new(batch_info, transition, sidecar, proof)
    }

    /// Generate a signed checkpoint payload for the current epoch.
    pub fn gen_signed_payload(&self) -> SignedCheckpointPayload {
        let payload = self.gen_payload();
        sign_checkpoint_payload(&payload, &self.sequencer)
    }

    /// Generate a signed checkpoint payload for a specific epoch.
    pub fn gen_signed_payload_for_epoch(&self, epoch: Epoch) -> SignedCheckpointPayload {
        let payload = self.gen_payload_for_epoch(epoch);
        sign_checkpoint_payload(&payload, &self.sequencer)
    }

    /// Advance to the next epoch and return new fixtures.
    pub fn next_epoch(self) -> Self {
        Self {
            epoch: self.epoch + 1,
            ..self
        }
    }
}

/// Sign a checkpoint payload with a sequencer keypair.
pub fn sign_checkpoint_payload(
    payload: &CheckpointPayload,
    sequencer: &SequencerKeypair,
) -> SignedCheckpointPayload {
    let hash = payload.compute_hash();
    let signature = sequencer.sign(&hash);
    SignedCheckpointPayload::new(payload.clone(), signature)
}

/// Generate random batch info for a given epoch.
pub fn gen_batch_info(epoch: Epoch) -> BatchInfo {
    let l1_range = gen_l1_block_range(100 + epoch as u64 * 10);
    let l2_range = gen_l2_block_range(epoch as u64 * 100);
    BatchInfo::new(epoch, l1_range, l2_range)
}

/// Generate random batch transition for a given epoch.
pub fn gen_batch_transition(epoch: Epoch) -> BatchTransition {
    let pre_state_root = gen_random_buf32();
    let post_state_root = gen_random_buf32();
    BatchTransition::new(epoch, pre_state_root, post_state_root)
}

/// Generate a checkpoint sidecar with random data.
pub fn gen_checkpoint_sidecar() -> CheckpointSidecar {
    // Generate small random state diff and empty logs for testing
    let mut rng = rand::thread_rng();
    let state_diff_len = rng.gen_range(100..500);
    let mut ol_state_diff = vec![0u8; state_diff_len];
    rng.fill_bytes(&mut ol_state_diff);

    // Empty OL logs for basic tests
    let ol_logs = Vec::new();

    CheckpointSidecar::new(ol_state_diff, ol_logs)
}

/// Generate a complete checkpoint payload for testing.
pub fn gen_checkpoint_payload(epoch: Epoch) -> CheckpointPayload {
    let batch_info = gen_batch_info(epoch);
    let transition = gen_batch_transition(epoch);
    let sidecar = gen_checkpoint_sidecar();
    let proof = gen_dummy_proof();

    CheckpointPayload::new(batch_info, transition, sidecar, proof)
}

/// Generate an L1 block range starting at the given height.
pub fn gen_l1_block_range(start_height: u64) -> L1BlockRange {
    let start = gen_l1_block_commitment(start_height);
    let end = gen_l1_block_commitment(start_height + 10);
    L1BlockRange::new(start, end)
}

/// Generate an L2 block range starting at the given slot.
pub fn gen_l2_block_range(start_slot: u64) -> L2BlockRange {
    let start = gen_l2_block_commitment(start_slot);
    let end = gen_l2_block_commitment(start_slot + 100);
    L2BlockRange::new(start, end)
}

/// Generate an L1 block commitment at the given height.
pub fn gen_l1_block_commitment(height: u64) -> L1BlockCommitment {
    let blkid = L1BlockId::from(gen_random_buf32());
    let l1_height = Height::from_consensus(height as u32).expect("valid height");
    L1BlockCommitment::new(l1_height, blkid)
}

/// Generate an L2 block commitment at the given slot.
pub fn gen_l2_block_commitment(slot: u64) -> L2BlockCommitment {
    let blkid = L2BlockId::from(gen_random_buf32());
    L2BlockCommitment::new(Slot::from(slot), blkid)
}

/// Generate a random 32-byte buffer.
pub fn gen_random_buf32() -> Buf32 {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    Buf32::from(bytes)
}

/// Generate a dummy proof (empty for testing without ZK verification).
pub fn gen_dummy_proof() -> Vec<u8> {
    // Empty proof for tests that don't verify proofs
    Vec::new()
}

#[cfg(test)]
mod tests {
    use strata_checkpoint_types_ssz::verify_checkpoint_payload_signature;
    use strata_identifiers::CredRule;

    use super::*;

    #[test]
    fn test_sequencer_keypair_signing() {
        let keypair = SequencerKeypair::random();
        let msg = gen_random_buf32();
        let sig = keypair.sign(&msg);

        // Verify signature is valid
        let cred_rule = CredRule::SchnorrKey(keypair.public_key);
        let payload = gen_checkpoint_payload(0);
        let signed = sign_checkpoint_payload(&payload, &keypair);

        assert!(verify_checkpoint_payload_signature(&signed, &cred_rule));

        // Signature should be 64 bytes
        assert_eq!(sig.as_ref().len(), 64);
    }

    #[test]
    fn test_checkpoint_fixtures_generation() {
        let fixtures = CheckpointFixtures::new();
        let payload = fixtures.gen_payload();

        assert_eq!(payload.epoch(), 0);
        assert!(!payload.sidecar().is_empty());
    }

    #[test]
    fn test_checkpoint_fixtures_epoch_progression() {
        let fixtures = CheckpointFixtures::new();
        assert_eq!(fixtures.epoch, 0);

        let fixtures = fixtures.next_epoch();
        assert_eq!(fixtures.epoch, 1);

        let payload = fixtures.gen_payload();
        assert_eq!(payload.epoch(), 1);
    }

    #[test]
    fn test_signed_payload_verification() {
        let fixtures = CheckpointFixtures::new();
        let signed = fixtures.gen_signed_payload();

        let cred_rule = CredRule::SchnorrKey(fixtures.sequencer.public_key);
        assert!(verify_checkpoint_payload_signature(&signed, &cred_rule));

        // Wrong key should fail
        let wrong_keypair = SequencerKeypair::random();
        let wrong_cred = CredRule::SchnorrKey(wrong_keypair.public_key);
        assert!(!verify_checkpoint_payload_signature(&signed, &wrong_cred));
    }
}
