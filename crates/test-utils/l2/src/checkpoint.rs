use k256::schnorr::SigningKey;
use rand::{thread_rng, Rng};
use ssz::Encode;
use strata_asm_common::{
    AsmHistoryAccumulatorState, AuxData, VerifiableManifestHash, VerifiedAuxData,
};
use strata_asm_txs_checkpoint::EnvelopeCheckpoint;
use strata_checkpoint_types_ssz::{
    compute_asm_manifests_hash_from_leaves, CheckpointClaim, CheckpointPayload, CheckpointSidecar,
    CheckpointTip, L2BlockRange, OLLog, TerminalHeaderComplement,
};
use strata_crypto::hash;
use strata_identifiers::{OLBlockCommitment, OLBlockId};
use strata_merkle::{CompactMmr64, Mmr, Sha256Hasher};
use strata_predicate::{PredicateKey, PredicateTypeId};
use strata_test_utils::ArbitraryGenerator;

/// Test harness for generating valid checkpoint payloads.
#[expect(
    missing_debug_implementations,
    reason = "contains private signing keys"
)]
pub struct CheckpointTestHarness {
    genesis_l1_height: u32,
    /// Raw secret key bytes for the sequencer identity.
    ///
    /// Stored so integration tests can reconstruct a Bitcoin keypair for SPS-51
    /// envelope signing where the envelope pubkey must match the sequencer predicate.
    sequencer_secret_key: [u8; 32],
    sequencer_pubkey: Vec<u8>,
    checkpoint_predicate: SigningKey,
    verified_tip: CheckpointTip,
}

impl CheckpointTestHarness {
    /// Creates a test harness with randomly generated keys and genesis state.
    ///
    /// Generates:
    /// - Random L1 genesis height (between 800,000 and 1,000,000)
    /// - Random sequencer and checkpoint signing keys
    /// - Genesis checkpoint tip at epoch 0
    pub fn new_random() -> Self {
        let mut rng = thread_rng();
        let genesis_l1_height: u32 = rng.gen_range(800_000..1_000_000);

        let genesis_ol_blkid = ArbitraryGenerator::new().generate();
        let genesis_blk = OLBlockCommitment::new(0, genesis_ol_blkid);

        let sequencer_key = SigningKey::random(&mut rng);
        let sequencer_secret_key = sequencer_key.to_bytes().into();
        let sequencer_pubkey = sequencer_key.verifying_key().to_bytes().to_vec();
        let checkpoint_predicate = SigningKey::random(&mut rng);

        let genesis_tip = CheckpointTip::new(0, genesis_l1_height, genesis_blk);
        Self {
            genesis_l1_height,
            sequencer_secret_key,
            sequencer_pubkey,
            checkpoint_predicate,
            verified_tip: genesis_tip,
        }
    }

    /// Creates a test harness with the given genesis L1 height and OL block ID.
    ///
    /// Generates random sequencer and checkpoint signing keys while using the
    /// provided genesis values. This ensures the harness's `verified_tip` matches
    /// the live ASM's checkpoint state when used in integration tests.
    pub fn new_with_genesis(genesis_l1_height: u32, genesis_ol_blkid: OLBlockId) -> Self {
        let mut rng = thread_rng();
        let genesis_blk = OLBlockCommitment::new(0, genesis_ol_blkid);

        let sequencer_key = SigningKey::random(&mut rng);
        let sequencer_secret_key = sequencer_key.to_bytes().into();
        let sequencer_pubkey = sequencer_key.verifying_key().to_bytes().to_vec();
        let checkpoint_predicate = SigningKey::random(&mut rng);

        let genesis_tip = CheckpointTip::new(0, genesis_l1_height, genesis_blk);
        Self {
            genesis_l1_height,
            sequencer_secret_key,
            sequencer_pubkey,
            checkpoint_predicate,
            verified_tip: genesis_tip,
        }
    }

    pub fn sequencer_predicate(&self) -> PredicateKey {
        PredicateKey::new(
            PredicateTypeId::Bip340Schnorr,
            self.sequencer_pubkey.clone(),
        )
    }

    pub fn checkpoint_predicate(&self) -> PredicateKey {
        PredicateKey::new(
            PredicateTypeId::Bip340Schnorr,
            self.checkpoint_predicate
                .verifying_key()
                .to_bytes()
                .to_vec(),
        )
    }

    /// Returns the sequencer's x-only public key bytes (used as envelope pubkey).
    pub fn sequencer_pubkey(&self) -> &[u8] {
        &self.sequencer_pubkey
    }

    /// Returns the sequencer's raw secret key bytes.
    ///
    /// Used by integration tests to construct a Bitcoin keypair for SPS-51 envelope
    /// transactions where the taproot pubkey must match the sequencer predicate.
    pub fn sequencer_secret_key(&self) -> &[u8; 32] {
        &self.sequencer_secret_key
    }

    pub fn genesis_l1_height(&self) -> u32 {
        self.genesis_l1_height
    }

    pub fn verified_tip(&self) -> &CheckpointTip {
        &self.verified_tip
    }

    /// Generates a new checkpoint tip that advances from the current verified tip.
    ///
    /// The new tip will:
    /// - Increment the epoch by 1
    /// - Process 1-100 random L1 blocks
    /// - Process 1-200 random L2 blocks
    pub fn gen_new_tip(&self) -> CheckpointTip {
        let mut rng = thread_rng();
        let mut arb = ArbitraryGenerator::new();
        let l1_blocks_processed: u32 = rng.gen_range(1..=100);
        let ol_blocks_processed: u64 = rng.gen_range(1..=200);

        let verified_tip = self.verified_tip;

        let new_epoch = verified_tip.epoch + 1;
        let new_covered_l1_height = verified_tip.l1_height + l1_blocks_processed;
        let new_ol_slot = verified_tip.l2_commitment().slot() + ol_blocks_processed;
        let new_ol_blkid: OLBlockId = arb.generate();
        let new_ol_block_commitment = OLBlockCommitment::new(new_ol_slot, new_ol_blkid);

        CheckpointTip::new(new_epoch, new_covered_l1_height, new_ol_block_commitment)
    }

    /// Updates the verified tip to reflect a newly accepted checkpoint.
    pub fn update_verified_tip(&mut self, new_tip: CheckpointTip) {
        self.verified_tip = new_tip
    }

    /// Generates deterministic manifest leaves for L1 blocks between verified tip and new tip.
    ///
    /// Each leaf is a hash derived from the L1 block height, ensuring reproducible test data.
    fn gen_manifest_leaves(&self, new_tip: &CheckpointTip) -> Vec<[u8; 32]> {
        let start_height = self.verified_tip.l1_height() + 1;
        let end_height = new_tip.l1_height;
        (start_height..=end_height)
            .map(|i| {
                let seed = format!("random_leaf_{}", i);
                hash::raw(seed.as_bytes()).0
            })
            .collect()
    }

    /// Generates verified auxiliary data containing ASM manifest hashes with MMR proofs.
    ///
    /// Constructs a complete ASM history accumulator state with manifests for all L1 blocks
    /// from genesis to the new tip, including Merkle proofs for each manifest hash.
    pub fn gen_verified_aux(&self, new_tip: &CheckpointTip) -> VerifiedAuxData {
        let leaves = self.gen_manifest_leaves(new_tip);
        let mut proof_list = Vec::new();

        let mut manifest_mmr = CompactMmr64::new(64);
        let mut asm_accumulator_state =
            AsmHistoryAccumulatorState::new(self.genesis_l1_height as u64);

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

    /// Generates a valid checkpoint payload with a randomly generated tip.
    ///
    /// Convenience wrapper around [`Self::build_payload_with_tip`] that automatically
    /// generates a new checkpoint tip advancing from the current verified tip.
    pub fn build_payload(&self) -> CheckpointPayload {
        let new_tip = self.gen_new_tip();
        self.build_payload_with_tip(new_tip)
    }

    /// Generates a valid checkpoint payload signed by the checkpoint predicate.
    ///
    /// Creates a complete checkpoint payload including:
    /// - Random state diff and empty OL logs in the sidecar
    /// - Properly constructed checkpoint claim with manifest hashes
    /// - Valid checkpoint proof signature
    pub fn build_payload_with_tip(&self, new_tip: CheckpointTip) -> CheckpointPayload {
        use k256::{ecdsa::signature::SignatureEncoding, schnorr::signature::Signer};

        let state_diff: Vec<u8> = ArbitraryGenerator::new().generate();
        let ol_logs = Vec::new();
        let mut arb = ArbitraryGenerator::new();
        let terminal_header_complement = TerminalHeaderComplement::new(
            thread_rng().gen(),
            arb.generate(),
            arb.generate(),
            arb.generate(),
        );
        let terminal_header_complement_hash = terminal_header_complement.compute_hash();
        let sidecar = CheckpointSidecar::new(
            state_diff.clone(),
            ol_logs.clone(),
            terminal_header_complement,
        )
        .unwrap();

        let state_diff_hash = hash::raw(&state_diff).into();
        let ol_logs_hash = hash::raw(&ol_logs.as_ssz_bytes()).into();

        let manifest_hashes = self.gen_manifest_leaves(&new_tip);
        let asm_manifests_hash = compute_asm_manifests_hash_from_leaves(&manifest_hashes);

        let l2_range = L2BlockRange::new(self.verified_tip.l2_commitment, new_tip.l2_commitment);
        let claim = CheckpointClaim::new(
            new_tip.epoch,
            l2_range,
            asm_manifests_hash,
            state_diff_hash,
            ol_logs_hash,
            terminal_header_complement_hash,
        );

        let proof = self
            .checkpoint_predicate
            .sign(&claim.as_ssz_bytes())
            .to_vec();

        CheckpointPayload::new(new_tip, sidecar, proof).unwrap()
    }

    /// Generates a valid checkpoint payload with custom OL logs and externally provided
    /// manifest hashes.
    ///
    /// Unlike [`Self::build_payload_with_tip`] which uses empty OL logs and internally
    /// generated manifest hashes, this method accepts OL logs (e.g., containing withdrawal
    /// intents) and manifest hashes obtained from the live ASM's MMR.
    pub fn build_payload_with_tip_and_logs(
        &self,
        new_tip: CheckpointTip,
        ol_logs: Vec<OLLog>,
        manifest_hashes: &[[u8; 32]],
    ) -> CheckpointPayload {
        use k256::{ecdsa::signature::SignatureEncoding, schnorr::signature::Signer};

        let state_diff: Vec<u8> = ArbitraryGenerator::new().generate();
        let mut arb = ArbitraryGenerator::new();
        let terminal_header_complement = TerminalHeaderComplement::new(
            thread_rng().gen(),
            arb.generate(),
            arb.generate(),
            arb.generate(),
        );
        let terminal_header_complement_hash = terminal_header_complement.compute_hash();
        let sidecar = CheckpointSidecar::new(
            state_diff.clone(),
            ol_logs.clone(),
            terminal_header_complement,
        )
        .unwrap();

        let state_diff_hash = hash::raw(&state_diff).into();
        let ol_logs_hash = hash::raw(&ol_logs.as_ssz_bytes()).into();

        let asm_manifests_hash = compute_asm_manifests_hash_from_leaves(manifest_hashes);

        let l2_range = L2BlockRange::new(self.verified_tip.l2_commitment, new_tip.l2_commitment);
        let claim = CheckpointClaim::new(
            new_tip.epoch,
            l2_range,
            asm_manifests_hash,
            state_diff_hash,
            ol_logs_hash,
            terminal_header_complement_hash,
        );

        let proof = self
            .checkpoint_predicate
            .sign(&claim.as_ssz_bytes())
            .to_vec();

        CheckpointPayload::new(new_tip, sidecar, proof).unwrap()
    }

    /// Wraps a checkpoint payload into an [`EnvelopeCheckpoint`] with the sequencer's pubkey.
    ///
    /// This simulates the extraction that would happen on the ASM side when parsing
    /// an SPS-51 envelope transaction where the sequencer's pubkey is used as the
    /// taproot key.
    pub fn wrap_in_envelope(&self, payload: CheckpointPayload) -> EnvelopeCheckpoint {
        EnvelopeCheckpoint {
            payload,
            envelope_pubkey: self.sequencer_pubkey.clone(),
        }
    }
}
