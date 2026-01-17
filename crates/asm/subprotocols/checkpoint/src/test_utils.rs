use k256::schnorr::{Signature, SigningKey, VerifyingKey, signature::Signer};
use rand::thread_rng;
use ssz::Encode;
use strata_asm_common::{
    AsmHistoryAccumulatorState, AuxData, VerifiableManifestHash, VerifiedAuxData,
};
use strata_checkpoint_types_ssz::CheckpointPayload;
use strata_crypto::hash;
use strata_merkle::{CompactMmr64, Mmr, Sha256Hasher};
use strata_predicate::{PredicateKey, PredicateTypeId};
use strata_test_utils::ArbitraryGenerator;

use crate::state::CheckpointConfig;

#[derive(Debug)]
pub struct SequencerKeypair {
    sk_bytes: [u8; 32],
    vk: VerifyingKey,
}

impl SequencerKeypair {
    /// Generate a random sequencer keypair.
    pub fn random() -> Self {
        let sk = SigningKey::random(&mut thread_rng());
        let vk = *sk.verifying_key();
        let sk_bytes = sk.to_bytes().into();
        Self { sk_bytes, vk }
    }

    /// Get the sequencer predicate for this keypair.
    pub fn predicate(&self) -> PredicateKey {
        PredicateKey::new(PredicateTypeId::Bip340Schnorr, self.vk.to_bytes().to_vec())
    }

    /// Sign a checkpoint payload with the sequencer key.
    ///
    /// Signs the raw SSZ-encoded payload bytes (not pre-hashed).
    /// This is compatible with the predicate framework's Schnorr BIP-340 verifier.
    pub fn sign(&self, payload: &CheckpointPayload) -> Signature {
        let signing_key =
            SigningKey::from_bytes(&self.sk_bytes).expect("secret key bytes should be valid");
        let payload_bytes = payload.as_ssz_bytes();
        signing_key.sign(&payload_bytes)
    }
}

pub fn setup_verified_aux_data(genesis_height: u64, count: usize) -> VerifiedAuxData {
    // Generate random leaves by hashing incrementing values
    let leaves: Vec<[u8; 32]> = (0..count)
        .map(|i| {
            let seed = format!("random_leaf_{}", i);
            hash::raw(seed.as_bytes()).0
        })
        .collect();
    let mut proof_list = Vec::new();

    let mut manifest_mmr = CompactMmr64::new(64);
    let mut asm_accumulator_state = AsmHistoryAccumulatorState::new(genesis_height);

    for leaf in &leaves {
        asm_accumulator_state.add_manifest_leaf(*leaf).unwrap();

        let proof1 = Mmr::<Sha256Hasher>::add_leaf_updating_proof_list(
            &mut manifest_mmr,
            *leaf,
            &mut proof_list,
        )
        .unwrap();
        proof_list.push(proof1);
    }

    let manifest_hashes = leaves
        .iter()
        .zip(proof_list)
        .map(|(leaf, proof)| VerifiableManifestHash::new(*leaf, proof))
        .collect();

    let data = AuxData::new(manifest_hashes, vec![]);
    VerifiedAuxData::try_new(&data, &asm_accumulator_state).unwrap()
}

pub fn setup_genesis(genesis_l1_height: u32) -> (CheckpointConfig, SequencerKeypair) {
    let keypair = SequencerKeypair::random();
    let sequencer_predicate = keypair.predicate();
    let genesis_ol_blkid = ArbitraryGenerator::new().generate();
    let config = CheckpointConfig {
        sequencer_predicate,
        checkpoint_predicate: PredicateKey::always_accept(),
        genesis_l1_height,
        genesis_ol_blkid,
    };
    (config, keypair)
}
