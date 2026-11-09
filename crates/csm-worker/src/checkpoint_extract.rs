//! Locate the L1 transaction carrying a validated checkpoint.

use bitcoin::{Block, hashes::Hash};
use strata_asm_checkpoint_types::{
    AsmManifestRangeHash, CheckpointPayload, CheckpointTip, compute_asm_manifests_hash_from_leaves,
};
use strata_asm_common::{TxInputRef, VerifiedAuxData};
use strata_asm_proto_checkpoint::{
    CheckpointEnvelopeError, CheckpointState, authenticate_checkpoint_envelope,
};
use strata_asm_proto_checkpoint_txs::{
    CHECKPOINT_SUBPROTOCOL_ID, OL_STF_CHECKPOINT_TX_TYPE, decode_checkpoint_payload,
    parse_checkpoint_envelope,
};
use strata_checkpoint_verification::{CheckpointL1Range, verify_progression};
use strata_identifiers::RBuf32;
use strata_l1_txfmt::{MagicBytes, ParseConfig};
use tracing::{debug, info};

use crate::errors::CsmWorkerResult;

/// Identification of the L1 transaction that carries a validated checkpoint,
/// along with its decoded payload.
///
/// `txid` and `wtxid` use [`RBuf32`] so their `Debug`/`Display` follow Bitcoin's
/// reversed-byte hash convention.
#[derive(Debug, Clone)]
pub(crate) struct ExtractedCheckpoint {
    pub(crate) txid: RBuf32,
    pub(crate) wtxid: RBuf32,
    pub(crate) payload: CheckpointPayload,
}

///  Checkpoint related data shared across every checkpoint tip log in one ASM block.
pub(crate) struct CheckpointVerificationContext {
    /// L1 block whose txs carry the checkpoint envelopes.
    pub(crate) block: Block,
    /// MMR-verified manifest hashes and spent outputs requested by ASM.
    /// Pre-upgrade aux may omit spent outputs; see [`extract_matching_checkpoint`].
    pub(crate) verified_aux_data: VerifiedAuxData,
    /// Advances past each accepted checkpoint, mirroring ASM.
    pub(crate) checkpoint_state: CheckpointState,
    /// Next `block.txdata` index to scan; advances past each matched tx.
    pub(crate) scan_cursor: usize,
}

/// Returns the SPS-50-tagged checkpoint envelope tx in `ctx.block` that ASM
/// accepted for `expected`, or `None` if none does.
///
/// Uses ASM's authentication, except that missing spent-output aux falls back to key-only
/// matching for pre-upgrade replay. Parsing still requires a TapScript leaf, and available
/// spent outputs still undergo the P2TR check, even for pre-upgrade blocks. This relies on
/// the asm#322 rollout prerequisite: a per-network scan finding no historical carriers that
/// violate the new authentication rules. It does not reproduce every pre-upgrade ASM rule.
///
/// Resumes from `ctx.scan_cursor`; on a successful match the cursor is bumped
/// past the matched tx and `ctx.checkpoint_state` is advanced.
pub(crate) fn extract_matching_checkpoint(
    ctx: &mut CheckpointVerificationContext,
    magic: MagicBytes,
    expected: &CheckpointTip,
    current_l1_height: u32,
) -> Option<ExtractedCheckpoint> {
    let parser = ParseConfig::new(magic);

    for (offset, tx) in ctx.block.txdata[ctx.scan_cursor..].iter().enumerate() {
        let Ok(tag) = parser.try_parse_tx(tx) else {
            continue;
        };
        if tag.subproto_id() != CHECKPOINT_SUBPROTOCOL_ID
            || tag.tx_type() != OL_STF_CHECKPOINT_TX_TYPE
        {
            continue;
        }

        let tx_input = TxInputRef::new(tx, tag);
        let spend = match parse_checkpoint_envelope(&tx_input) {
            Ok(spend) => spend,
            Err(e) => {
                debug!(
                    txid = %tx.compute_txid(),
                    error = %e,
                    "checkpoint envelope could not be parsed; skipping"
                );
                continue;
            }
        };
        let leaf = match authenticate_checkpoint_envelope(
            ctx.checkpoint_state.sequencer_key(),
            spend.clone(),
            &ctx.verified_aux_data,
        ) {
            Ok(leaf) => leaf,
            // This relies on ASM pre-process requesting the spent output for every parsed
            // envelope that passes verify_envelope_key under the parent state's key. New ASM
            // aux must honor those requests; the CSM cannot distinguish missing new aux from
            // aux stored before that request existed. MissingSpentOutput occurs only after
            // the key check passes, so reuse the parsed leaf for pre-upgrade replay.
            // The strict parser and any available spent-output check still apply; historical
            // compatibility relies on the no-violation scan described above.
            Err(CheckpointEnvelopeError::MissingSpentOutput { outpoint, .. }) => {
                info!(
                    txid = %tx.compute_txid(),
                    %outpoint,
                    "spent output absent from ASM aux; matching without it for pre-upgrade replay"
                );
                spend.unverified_leaf().clone()
            }
            Err(e) => {
                // Anyone can post an envelope, so a rejected one is not actionable.
                debug!(
                    txid = %tx.compute_txid(),
                    error = %e,
                    "checkpoint envelope not authenticated; skipping"
                );
                continue;
            }
        };
        let payload = match decode_checkpoint_payload(leaf.payload()) {
            Ok(payload) => payload,
            Err(e) => {
                debug!(
                    txid = %tx.compute_txid(),
                    error = %e,
                    "checkpoint payload could not be decoded; skipping"
                );
                continue;
            }
        };

        // Cheap pre-filter so we don't burn proof verification on payloads that
        // can't possibly match the tip ASM logged.
        let new_tip = payload.new_tip();
        if new_tip.epoch != expected.epoch || new_tip.l2_commitment() != expected.l2_commitment() {
            continue;
        }

        if let Err(e) = verify_checkpoint(
            &mut ctx.checkpoint_state,
            &payload,
            current_l1_height,
            &ctx.verified_aux_data,
        ) {
            debug!(
                txid = %tx.compute_txid(),
                epoch = expected.epoch,
                error = %e,
                "candidate checkpoint tx failed validation; skipping"
            );
            continue;
        }

        let txid = RBuf32::from(tx.compute_txid().to_byte_array());
        let wtxid = RBuf32::from(tx.compute_wtxid().to_byte_array());
        debug!(
            ?txid,
            ?wtxid,
            epoch = expected.epoch,
            "matched checkpoint tx"
        );
        ctx.scan_cursor += offset + 1;
        return Some(ExtractedCheckpoint {
            txid,
            wtxid,
            payload,
        });
    }

    None
}

/// Verifies an authenticated checkpoint payload mimicking how checkpoint subprotocol handles this.
/// This does not log any errors, just returns error indicating checkpoint validation failed.
fn verify_checkpoint(
    checkpoint_state: &mut CheckpointState,
    payload: &CheckpointPayload,
    current_l1_height: u32,
    verified_aux_data: &VerifiedAuxData,
) -> CsmWorkerResult<()> {
    let coverage = verify_progression(
        checkpoint_state.verified_tip(),
        payload.new_tip(),
        current_l1_height,
        checkpoint_state.next_transition(),
    )?;

    let asm_manifests_hash = match &coverage {
        CheckpointL1Range::Empty => AsmManifestRangeHash::ZERO,
        CheckpointL1Range::Range {
            start_height,
            end_height,
        } => {
            let manifest_hashes =
                verified_aux_data.get_manifest_hashes(*start_height as u64, *end_height as u64)?;
            compute_asm_manifests_hash_from_leaves(&manifest_hashes)
        }
    };
    checkpoint_state.advance(payload, asm_manifests_hash)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use bitcoin::{
        Amount, Block, BlockHash, CompactTarget, OutPoint, ScriptBuf, Sequence, Transaction, TxIn,
        TxMerkleNode, TxOut, Witness, WitnessProgram, WitnessVersion, XOnlyPublicKey,
        absolute::LockTime,
        block::{Header, Version as BlockVersion},
        consensus,
        hashes::{Hash, sha256d},
        key::UntweakedKeypair,
        secp256k1::{SECP256K1, schnorr::Signature},
        taproot::{LeafVersion, TaprootBuilder},
        transaction::Version,
    };
    use rand::{RngCore, rngs::OsRng};
    use strata_asm_checkpoint_types::{CheckpointPayload, CheckpointTip};
    use strata_asm_common::RawBitcoinTx;
    use strata_asm_params::CheckpointInitConfig;
    use strata_asm_proto_checkpoint_txs::OL_STF_CHECKPOINT_TX_TAG;
    use strata_asm_proto_txs_test_utils::{TEST_MAGIC_BYTES, create_dummy_tx};
    use strata_codec::encode_to_vec;
    use strata_codec_utils::CodecSsz;
    use strata_l1_envelope_fmt::EnvelopeScriptBuilder;
    use strata_l1_txfmt::ParseConfig;
    use strata_test_utils_checkpoint::CheckpointTestHarness;

    use super::*;

    /// Builds a transaction whose output 0 is locked by `script_pubkey`, for a
    /// checkpoint envelope tx to spend. `seed` keeps funding txids distinct.
    fn funding_tx(script_pubkey: ScriptBuf, seed: u32) -> Transaction {
        Transaction {
            version: Version::TWO,
            lock_time: LockTime::from_consensus(seed),
            input: vec![],
            output: vec![TxOut {
                value: Amount::from_sat(10_000),
                script_pubkey,
            }],
        }
    }

    /// A native P2TR locking script.
    fn p2tr_script() -> ScriptBuf {
        let program = WitnessProgram::new(WitnessVersion::V1, &[0x11; 32]).unwrap();
        ScriptBuf::new_witness_program(&program)
    }

    /// Verified aux data for `tip` that also carries the `funding` txs, as ASM
    /// would after requesting the outputs its envelope txs spend.
    fn aux_with_funding(
        harness: &CheckpointTestHarness,
        tip: &CheckpointTip,
        funding: &[&Transaction],
    ) -> VerifiedAuxData {
        let raw = funding
            .iter()
            .map(|tx| RawBitcoinTx::from_raw_bytes(consensus::serialize(tx)))
            .collect();
        harness.gen_verified_aux_with_txs(tip, raw)
    }

    /// Builds a reveal tx whose taproot envelope script embeds `envelope_pubkey`,
    /// carries the SSZ-encoded `payload` and spends output 0 of `funding`. Mirrors
    /// `create_reveal_transaction_stub` but lets callers control the envelope
    /// pubkey and spent output so they can simulate hostile-third-party,
    /// forged-signature and self-conflict cases.
    fn build_checkpoint_envelope_tx(
        payload: &CheckpointPayload,
        envelope_pubkey: &[u8],
        funding: &Transaction,
    ) -> Transaction {
        let payload_bytes = encode_to_vec(&CodecSsz::new(payload.clone())).expect("encode payload");
        let reveal_script = EnvelopeScriptBuilder::with_pubkey(envelope_pubkey)
            .expect("envelope builder")
            .add_envelope(&payload_bytes)
            .expect("add envelope")
            .build()
            .expect("build envelope script");

        let sps50_script = ParseConfig::new(TEST_MAGIC_BYTES)
            .encode_script_buf(&OL_STF_CHECKPOINT_TX_TAG.as_ref())
            .expect("encode SPS-50 script");

        // The taproot internal key is unrelated to the envelope pubkey embedded
        // in the script; pick a random one for the spend info.
        let mut seed = [0u8; 32];
        OsRng.fill_bytes(&mut seed);
        let internal_kp = UntweakedKeypair::from_seckey_slice(SECP256K1, &seed).unwrap();
        let internal_xonly = XOnlyPublicKey::from_keypair(&internal_kp).0;

        let taproot_spend_info = TaprootBuilder::new()
            .add_leaf(0, reveal_script.clone())
            .unwrap()
            .finalize(SECP256K1, internal_xonly)
            .expect("finalize taproot");

        let dummy_sig = Signature::from_slice(&[0u8; 64]).unwrap();
        let mut witness = Witness::new();
        witness.push(dummy_sig.as_ref());
        witness.push(reveal_script.clone());
        witness.push(
            taproot_spend_info
                .control_block(&(reveal_script, LeafVersion::TapScript))
                .expect("control block")
                .serialize(),
        );

        Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::new(funding.compute_txid(), 0),
                script_sig: ScriptBuf::new(),
                sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
                witness,
            }],
            output: vec![TxOut {
                value: Amount::ZERO,
                script_pubkey: sps50_script,
            }],
        }
    }

    /// Wraps `payload` in a reveal tx using the harness's sequencer pubkey and
    /// spending `funding`'s P2TR output (the legitimate, ASM-accepting envelope).
    fn legit_envelope_tx(
        harness: &CheckpointTestHarness,
        payload: &CheckpointPayload,
        funding: &Transaction,
    ) -> Transaction {
        build_checkpoint_envelope_tx(payload, harness.sequencer_pubkey(), funding)
    }

    /// Constructs a `CheckpointState` consistent with the harness's predicates
    /// and genesis. Mirrors what ASM has in its state at the pre-block point
    /// where it processes the first checkpoint tip.
    fn harness_checkpoint_state(harness: &CheckpointTestHarness) -> CheckpointState {
        let genesis_blkid = *harness.verified_tip().l2_commitment().blkid();
        let config = CheckpointInitConfig {
            sequencer_key: harness.sequencer_key(),
            checkpoint_predicate: harness.checkpoint_predicate(),
            genesis_l1_height: harness.genesis_l1_height(),
            genesis_ol_blkid: genesis_blkid,
        };
        CheckpointState::init(config)
    }

    fn block_from_txs(txs: Vec<Transaction>) -> Block {
        Block {
            header: Header {
                version: BlockVersion::TWO,
                prev_blockhash: BlockHash::from_raw_hash(sha256d::Hash::all_zeros()),
                merkle_root: TxMerkleNode::from_raw_hash(sha256d::Hash::all_zeros()),
                time: 0,
                bits: CompactTarget::from_consensus(0),
                nonce: 0,
            },
            txdata: txs,
        }
    }

    fn current_l1_height_for(tip: &CheckpointTip) -> u32 {
        // ASM passes the height of the L1 block being processed; the
        // checkpoint's new_tip.l1_height must be strictly less than this.
        tip.l1_height() + 1
    }

    fn make_ctx(
        block: Block,
        verified_aux_data: VerifiedAuxData,
        checkpoint_state: CheckpointState,
    ) -> CheckpointVerificationContext {
        CheckpointVerificationContext {
            block,
            verified_aux_data,
            checkpoint_state,
            scan_cursor: 0,
        }
    }

    /// Sanity check: a legit envelope tx alone is picked up and the txid/wtxid
    /// returned point at it.
    #[test]
    fn matches_single_checkpoint_tx() {
        let harness = CheckpointTestHarness::new_random();
        let payload = harness.build_payload();
        let expected = *payload.new_tip();
        let funding = funding_tx(p2tr_script(), 0);
        let tx = legit_envelope_tx(&harness, &payload, &funding);
        let mut ctx = make_ctx(
            block_from_txs(vec![create_dummy_tx(1, 1), tx.clone()]),
            aux_with_funding(&harness, &expected, &[&funding]),
            harness_checkpoint_state(&harness),
        );

        let result = extract_matching_checkpoint(
            &mut ctx,
            TEST_MAGIC_BYTES,
            &expected,
            current_l1_height_for(&expected),
        )
        .expect("legitimate checkpoint should validate");

        assert_eq!(result.payload.new_tip().epoch, expected.epoch);
        assert_eq!(result.txid, RBuf32::from(tx.compute_txid().to_byte_array()));
        assert_eq!(
            result.wtxid,
            RBuf32::from(tx.compute_wtxid().to_byte_array())
        );
    }

    /// A lagging CSM replays blocks the ASM processed before it requested spent outputs, so
    /// their aux data lacks the funding tx. The checkpoint that ASM accepted must still match.
    #[test]
    fn matches_checkpoint_from_aux_stored_before_spent_output_request() {
        let harness = CheckpointTestHarness::new_random();
        let payload = harness.build_payload();
        let expected = *payload.new_tip();
        let funding = funding_tx(p2tr_script(), 0);
        let tx = legit_envelope_tx(&harness, &payload, &funding);
        let mut ctx = make_ctx(
            block_from_txs(vec![tx.clone()]),
            aux_with_funding(&harness, &expected, &[]),
            harness_checkpoint_state(&harness),
        );

        let result = extract_matching_checkpoint(
            &mut ctx,
            TEST_MAGIC_BYTES,
            &expected,
            current_l1_height_for(&expected),
        )
        .expect("checkpoint accepted by the pre-upgrade ASM should match");

        assert_eq!(result.txid, RBuf32::from(tx.compute_txid().to_byte_array()));
    }

    /// If no envelope matches the expected tip on (epoch, l2_commitment),
    /// extraction returns None even though validation never gets to run.
    #[test]
    fn returns_none_when_no_envelope_matches_expected_tip() {
        let harness = CheckpointTestHarness::new_random();
        let payload = harness.build_payload();
        // Use the real payload's tip for aux construction (cheap), then craft
        // a foreign tip that differs on (epoch, l2_commitment) so the cheap
        // pre-filter rejects without ever invoking proof verification.
        let real_tip = *payload.new_tip();
        let foreign_tip = CheckpointTip::new(
            real_tip.epoch + 1,
            real_tip.l1_height() + 1,
            Default::default(),
        );
        let funding = funding_tx(p2tr_script(), 0);
        let mut ctx = make_ctx(
            block_from_txs(vec![legit_envelope_tx(&harness, &payload, &funding)]),
            aux_with_funding(&harness, &real_tip, &[&funding]),
            harness_checkpoint_state(&harness),
        );

        assert!(
            extract_matching_checkpoint(
                &mut ctx,
                TEST_MAGIC_BYTES,
                &foreign_tip,
                current_l1_height_for(&foreign_tip),
            )
            .is_none()
        );
    }

    /// A hostile third party publishes an envelope tx whose embedded pubkey is
    /// not the sequencer's. Its payload copies (epoch, l2_commitment) from the
    /// legit tip but has otherwise garbage sidecar/proof. The legit tx appears
    /// second in block order. ASM would reject the hostile tx (pubkey
    /// mismatch); our extractor must do the same and return the legit tx.
    #[test]
    fn rejects_hostile_envelope_with_wrong_pubkey() {
        let harness = CheckpointTestHarness::new_random();
        let payload = harness.build_payload();
        let expected = *payload.new_tip();

        // Hostile envelope: a separately generated harness signs the same tip
        // but the envelope pubkey is the hostile sequencer's, not the real one.
        // The payload has the matching (epoch, l2_commitment) by construction
        // because we ask the hostile harness to build for the same tip.
        let hostile = CheckpointTestHarness::new_with_genesis(
            harness.genesis_l1_height(),
            *harness.verified_tip().l2_commitment().blkid(),
        );
        let hostile_payload = hostile.build_payload_with_tip(expected);
        let hostile_funding = funding_tx(p2tr_script(), 1);
        let hostile_tx = build_checkpoint_envelope_tx(
            &hostile_payload,
            hostile.sequencer_pubkey(),
            &hostile_funding,
        );
        let funding = funding_tx(p2tr_script(), 0);
        let legit_tx = legit_envelope_tx(&harness, &payload, &funding);

        // ASM requests no spent output for the hostile envelope: its key fails first.
        let mut ctx = make_ctx(
            block_from_txs(vec![hostile_tx, legit_tx.clone()]),
            aux_with_funding(&harness, &expected, &[&funding]),
            harness_checkpoint_state(&harness),
        );

        let result = extract_matching_checkpoint(
            &mut ctx,
            TEST_MAGIC_BYTES,
            &expected,
            current_l1_height_for(&expected),
        )
        .expect("legit checkpoint should still be extracted past the hostile tx");

        assert_eq!(
            result.txid,
            RBuf32::from(legit_tx.compute_txid().to_byte_array()),
            "extractor must skip the hostile tx and pick the legit one"
        );
    }

    /// A buggy or malicious sequencer publishes two envelope txs in the same
    /// block: one carries a stale/garbage proof, the other is the real
    /// checkpoint ASM accepted. The bad tx appears first in block order. ASM
    /// rejects the bad tx (proof verification failure) and accepts the second.
    /// Our extractor must mirror that and return the second tx.
    #[test]
    fn rejects_sequencer_self_conflict_with_invalid_proof() {
        let harness = CheckpointTestHarness::new_random();
        let payload = harness.build_payload();
        let expected = *payload.new_tip();

        // Build a "bad" payload by reusing the new_tip but corrupting the
        // proof so it no longer verifies against the reconstructed claim.
        // Same (epoch, l2_commitment) as the legit one.
        let bad_payload = CheckpointPayload::new(
            *payload.new_tip(),
            payload.sidecar().clone(),
            vec![0xAB; payload.proof().len()],
        )
        .expect("payload");
        let bad_funding = funding_tx(p2tr_script(), 1);
        let bad_tx = legit_envelope_tx(&harness, &bad_payload, &bad_funding);
        let funding = funding_tx(p2tr_script(), 0);
        let legit_tx = legit_envelope_tx(&harness, &payload, &funding);
        assert_ne!(
            bad_tx.compute_txid(),
            legit_tx.compute_txid(),
            "txids must differ for the test to be meaningful"
        );

        let mut ctx = make_ctx(
            block_from_txs(vec![bad_tx, legit_tx.clone()]),
            aux_with_funding(&harness, &expected, &[&bad_funding, &funding]),
            harness_checkpoint_state(&harness),
        );

        let result = extract_matching_checkpoint(
            &mut ctx,
            TEST_MAGIC_BYTES,
            &expected,
            current_l1_height_for(&expected),
        )
        .expect("legit checkpoint should still be extracted past the invalid one");

        assert_eq!(
            result.txid,
            RBuf32::from(legit_tx.compute_txid().to_byte_array()),
            "extractor must skip the invalid-proof tx and pick the legit one"
        );
    }

    /// Anyone who gets a non-standard tx mined can spend a segwit v2 output with
    /// any witness, so a forged envelope carrying the sequencer pubkey and a valid
    /// proof for the same tip lands ahead of the real one. ASM rejects it because
    /// Bitcoin never checked its signature; the extractor must pick the real one.
    #[test]
    fn rejects_sequencer_envelope_spending_unencumbered_output() {
        let harness = CheckpointTestHarness::new_random();
        let payload = harness.build_payload();
        let expected = *payload.new_tip();

        let witness_v2 = WitnessProgram::new(WitnessVersion::V2, &[0x11; 32]).unwrap();
        let forged_funding = funding_tx(ScriptBuf::new_witness_program(&witness_v2), 1);
        let forged_tx = legit_envelope_tx(&harness, &payload, &forged_funding);
        let funding = funding_tx(p2tr_script(), 0);
        let legit_tx = legit_envelope_tx(&harness, &payload, &funding);

        // ASM requests both spent outputs: both envelopes carry the sequencer key.
        let mut ctx = make_ctx(
            block_from_txs(vec![forged_tx, legit_tx.clone()]),
            aux_with_funding(&harness, &expected, &[&forged_funding, &funding]),
            harness_checkpoint_state(&harness),
        );

        let result = extract_matching_checkpoint(
            &mut ctx,
            TEST_MAGIC_BYTES,
            &expected,
            current_l1_height_for(&expected),
        )
        .expect("legit checkpoint should still be extracted past the forged one");

        assert_eq!(
            result.txid,
            RBuf32::from(legit_tx.compute_txid().to_byte_array()),
            "extractor must skip the forged tx and pick the legit one"
        );
    }

    /// Two consecutive valid envelope txs in one block. The shared
    /// `CheckpointState` must advance between calls so the second one verifies.
    #[test]
    fn advances_state_across_consecutive_checkpoints() {
        let mut harness = CheckpointTestHarness::new_random();
        let initial_tip = *harness.verified_tip();
        let state = harness_checkpoint_state(&harness);

        let payload_1 = harness.build_payload();
        let tip_1 = *payload_1.new_tip();
        let funding_1 = funding_tx(p2tr_script(), 1);
        let tx_1 = legit_envelope_tx(&harness, &payload_1, &funding_1);

        harness.update_verified_tip(tip_1);
        let payload_2 = harness.build_payload();
        let tip_2 = *payload_2.new_tip();
        let funding_2 = funding_tx(p2tr_script(), 2);
        let tx_2 = legit_envelope_tx(&harness, &payload_2, &funding_2);

        // Rewind so `gen_verified_aux` covers the union of both payloads' L1
        // sub-ranges in one aux blob.
        harness.update_verified_tip(initial_tip);
        let mut ctx = make_ctx(
            block_from_txs(vec![tx_1.clone(), tx_2.clone()]),
            aux_with_funding(&harness, &tip_2, &[&funding_1, &funding_2]),
            state,
        );
        let current_l1_height = current_l1_height_for(&tip_2);

        let result_1 =
            extract_matching_checkpoint(&mut ctx, TEST_MAGIC_BYTES, &tip_1, current_l1_height)
                .expect("first checkpoint should validate");
        assert_eq!(
            result_1.txid,
            RBuf32::from(tx_1.compute_txid().to_byte_array())
        );
        assert_eq!(ctx.scan_cursor, 1, "cursor should advance past tx_1");

        let result_2 =
            extract_matching_checkpoint(&mut ctx, TEST_MAGIC_BYTES, &tip_2, current_l1_height)
                .expect("second checkpoint must validate against the advanced state");
        assert_eq!(
            result_2.txid,
            RBuf32::from(tx_2.compute_txid().to_byte_array())
        );
        assert_eq!(ctx.scan_cursor, 2, "cursor should advance past tx_2");
    }
}
