//! Guest-side DA correctness checks for the EE outer (acct) proof.
//!
//! Sits on top of [`strata_ee_acct_runtime::verify_and_process_update`]
//! and adds the DA-binding checks the outer proof enforces:
//!
//! - reassemble the published [`DaBlob`] from reveal-tx envelope payloads and bind its `batch_id`
//!   to the chunk transitions under proof;
//! - verify each reveal-tx wtxid is included in an L1 block whose header chains up to the public
//!   `l1_block_hash`, and that the coinbase tx is itself in that block;
//! - bind `da_witness.l1_block_hash` to the highest-idx [`LedgerRefs`] claim (via
//!   [`bind_da_witness_to_ledger_refs`]).
//!
//! # Deferred work: state-diff consistency
//!
//! The `DaBlob.state_diff` carried on L1 is reassembled and `batch_id`-bound,
//! but the outer proof does **not** yet verify that applying it to the
//! pre-state produces the post-state the chunk transitions converged on.
//! Closing that gap requires:
//!
//! - `ChunkTransition` to expose the post-execution state root in its SSZ pubvals (today it carries
//!   only `parent_exec_blkid`/`tip_exec_blkid`),
//! - the host to populate `EePrivateInput::raw_partial_pre_state` with the sparse-MPT witness
//!   `BatchStateDiff::apply` consumes (today an empty placeholder; sourced from the same
//!   `RangeWitnessExtractor` the chunk pipeline already uses).
//!
//! Both items belong on a follow-up branch coordinated with a chunk SP1
//! ELF redeploy + chunk predicate key rotation.
//
// FIXME(#1751): the chunked-envelope wire format is being redesigned in
// alpenlabs/alpen#1751. When it lands, the reassembly path here will
// need to source `blob_hash` from the commit tx's OP_RETURN (instead of
// per-chunk headers), verify the commit tx's inclusion in its L1 block,
// and add a schnorr signature check on each reveal under `sequencer_pk`.
// The wtxid Merkle proof against the witness-commitment in the coinbase
// keeps working — that part survives.

use alpen_ee_common::{DaBlob, ReassemblyError, reassemble_da_blob};
use bitcoin::{
    Transaction, block::Header as BitcoinHeader, consensus::deserialize as btc_deserialize,
    hashes::Hash as _,
};
use ssz::Decode;
use strata_acct_types::Hash;
use strata_ee_acct_runtime::{ArchivedDaBlockWitness, ArchivedDaWitness, ArchivedEePrivateInput};
use strata_ee_chain_types::ChunkTransition;
use strata_snark_acct_types::LedgerRefs;

use crate::da_inclusion::{
    DaInclusionError, verify_coinbase_inclusion, verify_header_chain, verify_wtxid_inclusion,
};

/// Detail payload for [`DaVerificationError::BatchIdMismatch`].
///
/// Boxed inside the enum so the overall `Result` size stays small —
/// each [`Hash`] is 32 bytes and four of them inline would push the
/// error past clippy's `result_large_err` threshold.
#[derive(Debug)]
pub struct BatchIdMismatch {
    pub blob_prev: Hash,
    pub blob_last: Hash,
    pub expected_prev: Hash,
    pub expected_last: Hash,
}

/// Errors raised by [`verify_da_witness`].
#[derive(Debug, thiserror::Error)]
pub enum DaVerificationError {
    #[error("batch under proof has no chunks; cannot derive batch id")]
    NoChunks,
    #[error("first chunk transition decode failed: {0:?}")]
    FirstChunkDecode(ssz::DecodeError),
    #[error("last chunk transition decode failed: {0:?}")]
    LastChunkDecode(ssz::DecodeError),
    #[error("DA blob reassembly failed: {0}")]
    Reassembly(#[from] ReassemblyError),
    #[error(
        "DA blob batch_id mismatch: \
         blob=(prev={:?}, last={:?}), \
         expected=(prev={:?}, last={:?})",
        .0.blob_prev, .0.blob_last, .0.expected_prev, .0.expected_last
    )]
    BatchIdMismatch(Box<BatchIdMismatch>),
    #[error("DA inclusion check failed: {0}")]
    Inclusion(#[from] DaInclusionError),
    #[error("malformed coinbase tx in DA witness: {0}")]
    CoinbaseDecode(String),
    #[error(
        "DaWitness.l1_block_hash does not match highest-idx LedgerRef: \
         witness_tip={witness_tip:?}, ledger_tip={ledger_tip:?}"
    )]
    LedgerTipMismatch {
        witness_tip: [u8; 32],
        ledger_tip: [u8; 32],
    },
    #[error("DA blocks present but LedgerRefs is empty")]
    LedgerRefsEmpty,
}

/// Runs the DA-correctness checks the outer proof currently enforces.
///
/// Returns the reassembled [`DaBlob`] on success so the deferred
/// state-diff consistency check (see module-level note) can use it
/// without re-decoding once it lands.
///
/// Skips reassembly entirely when the witness carries no DA blocks —
/// keeps zero-DA batches (e.g. genesis or perf fixtures) working
/// without requiring the host to plumb witnesses unconditionally.
pub fn verify_da_witness(
    ee_input: &ArchivedEePrivateInput,
    da_witness: &ArchivedDaWitness,
) -> Result<Option<DaBlob>, DaVerificationError> {
    if da_witness.blocks().is_empty() {
        return Ok(None);
    }

    let blob = reassemble_and_bind_batch_id(ee_input, da_witness)?;

    let l1_block_hash = *da_witness.l1_block_hash();
    for block in da_witness.blocks() {
        verify_block_inclusion(block, l1_block_hash)?;
    }

    Ok(Some(blob))
}

/// Cross-checks that `DaWitness.l1_block_hash` (the tip the in-proof
/// inclusion checks anchored to) matches the highest-idx entry in
/// [`LedgerRefs`] (the tip the OL canonicality check will anchor to).
///
/// Without this binding, a host could pass a `DaWitness` anchored to
/// one tip and a [`LedgerRefs`] anchored to another, in which case
/// inclusion verifies against tip A while OL canonicality verifies
/// tip B — defeating the intended chain-of-custody.
pub fn bind_da_witness_to_ledger_refs(
    da_witness: &ArchivedDaWitness,
    ledger_refs: &LedgerRefs,
) -> Result<(), DaVerificationError> {
    if da_witness.blocks().is_empty() {
        // No DA on this batch ⇒ no LedgerRefs expected. Empty
        // witness + empty refs is the only valid combo here; we
        // don't fail if the host happened to populate one and not
        // the other for batches with no DA.
        return Ok(());
    }

    let claims = ledger_refs.l1_header_refs();
    let tip_claim = claims
        .iter()
        .max_by_key(|c| c.idx())
        .ok_or(DaVerificationError::LedgerRefsEmpty)?;
    let ledger_tip: [u8; 32] = tip_claim.entry_hash().into();
    let witness_tip = *da_witness.l1_block_hash();
    if witness_tip != ledger_tip {
        return Err(DaVerificationError::LedgerTipMismatch {
            witness_tip,
            ledger_tip,
        });
    }
    Ok(())
}

/// Reassembles the [`DaBlob`] from reveal envelope payloads and asserts
/// its `batch_id` matches the `(first_chunk.parent, last_chunk.tip)` of
/// the batch under proof.
fn reassemble_and_bind_batch_id(
    ee_input: &ArchivedEePrivateInput,
    da_witness: &ArchivedDaWitness,
) -> Result<DaBlob, DaVerificationError> {
    let encoded_chunks = collect_encoded_chunks(da_witness);
    let blob = reassemble_da_blob(&encoded_chunks)?;

    let (expected_prev, expected_last) = derive_batch_endpoints_from_chunks(ee_input)?;
    let blob_prev = blob.batch_id.prev_block();
    let blob_last = blob.batch_id.last_block();
    if blob_prev != expected_prev || blob_last != expected_last {
        return Err(DaVerificationError::BatchIdMismatch(Box::new(
            BatchIdMismatch {
                blob_prev,
                blob_last,
                expected_prev,
                expected_last,
            },
        )));
    }
    Ok(blob)
}

/// Per-block inclusion checks — header chain to tip, coinbase Merkle
/// proof against `header.merkle_root`, and one wtxid Merkle proof per
/// reveal against the witness root committed in the coinbase OP_RETURN.
fn verify_block_inclusion(
    block: &ArchivedDaBlockWitness,
    l1_block_hash: [u8; 32],
) -> Result<(), DaVerificationError> {
    // 1. Header chain: parse start header, walk to tip.
    let chain_to_tip = block.raw_header_chain_to_tip().iter().map(|v| v.as_ref());
    verify_header_chain(block.raw_header(), chain_to_tip, l1_block_hash)?;

    // 2. Parse start header for merkle_root, then verify coinbase Merkle proof against it.
    let start_header: BitcoinHeader = btc_deserialize(block.raw_header()).map_err(|e| {
        DaVerificationError::Inclusion(DaInclusionError::MalformedHeader(e.to_string()))
    })?;
    let merkle_root = start_header.merkle_root.to_byte_array();

    let coinbase_raw = block.raw_coinbase_tx();
    let coinbase: Transaction = btc_deserialize(coinbase_raw)
        .map_err(|e| DaVerificationError::CoinbaseDecode(e.to_string()))?;
    let coinbase_txid = coinbase.compute_txid().to_byte_array();
    let cb_proof = block.coinbase_to_header_proof();
    verify_coinbase_inclusion(
        coinbase_txid,
        cb_proof.siblings(),
        cb_proof.position(),
        merkle_root,
    )?;

    // 3. Per-reveal wtxid inclusion under the coinbase witness commitment.
    for reveal in block.reveals() {
        let proof = reveal.wtxid_to_witness_root_proof();
        verify_wtxid_inclusion(
            *reveal.wtxid(),
            proof.siblings(),
            proof.position(),
            coinbase_raw,
        )?;
    }

    Ok(())
}

/// Flattens all reveal envelope payloads across all DA blocks into the
/// `header || payload` byte buffers that
/// [`alpen_ee_common::reassemble_da_blob`] consumes.
fn collect_encoded_chunks(da_witness: &ArchivedDaWitness) -> Vec<Vec<u8>> {
    da_witness
        .blocks()
        .iter()
        .flat_map(|b| b.reveals().iter())
        .map(|r| r.envelope_payload().to_vec())
        .collect()
}

/// Derives `(prev_block, last_block)` for the batch under proof from
/// the first and last [`ChunkTransition`]s. Mirrors the implicit
/// `BatchId::from_parts(prev, last)` convention enforced today by
/// [`alpen_ee_common::Batch::id`].
fn derive_batch_endpoints_from_chunks(
    ee_input: &ArchivedEePrivateInput,
) -> Result<(Hash, Hash), DaVerificationError> {
    let chunks = ee_input.chunks();
    if chunks.is_empty() {
        return Err(DaVerificationError::NoChunks);
    }

    let first = ChunkTransition::from_ssz_bytes(chunks[0].chunk_transition_ssz())
        .map_err(DaVerificationError::FirstChunkDecode)?;
    let last_idx = chunks.len() - 1;
    let last = ChunkTransition::from_ssz_bytes(chunks[last_idx].chunk_transition_ssz())
        .map_err(DaVerificationError::LastChunkDecode)?;

    Ok((first.parent_exec_blkid(), last.tip_exec_blkid()))
}

#[cfg(test)]
mod tests {
    use alpen_ee_common::{BatchId, DaBlob, EvmHeaderSummary, prepare_da_chunks};
    use alpen_reth_statediff::BatchStateDiff;
    use rkyv::rancor::Error as RkyvError;
    use strata_acct_types::Hash;
    use strata_ee_acct_runtime::{
        ArchivedDaWitness, ArchivedEePrivateInput, BitcoinMerkleProof, ChunkInput, DaBlockWitness,
        DaWitness, EePrivateInput, RevealWitness,
    };
    use strata_ee_chain_types::{ChunkTransition, ExecInputs, ExecOutputs};

    use super::*;

    fn fake_chunk_transition(parent: Hash, tip: Hash) -> ChunkTransition {
        ChunkTransition::new(
            parent,
            tip,
            ExecInputs::new_empty(),
            ExecOutputs::new_empty(),
        )
    }

    fn fake_da_blob(prev: Hash, last: Hash) -> DaBlob {
        DaBlob {
            batch_id: BatchId::from_parts(prev, last),
            evm_header: EvmHeaderSummary {
                block_num: 1,
                timestamp: 1_700_000_000,
                base_fee: 1_000_000_000,
                gas_used: 21_000,
                gas_limit: 30_000_000,
            },
            state_diff: BatchStateDiff::default(),
        }
    }

    fn build_da_witness_for_blob(blob: &DaBlob) -> DaWitness {
        let chunks = prepare_da_chunks(blob).expect("encode chunks");
        let reveals = chunks
            .into_iter()
            .enumerate()
            .map(|(i, payload)| {
                RevealWitness::new(
                    [i as u8; 32],
                    [(i as u8).wrapping_add(0x80); 32],
                    BitcoinMerkleProof::default(),
                    payload,
                )
            })
            .collect();
        let block = DaBlockWitness::new(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            BitcoinMerkleProof::default(),
            reveals,
        );
        DaWitness::new([0xee; 32], vec![block])
    }

    fn build_ee_input_with_endpoints(prev: Hash, last: Hash) -> EePrivateInput {
        let transition = fake_chunk_transition(prev, last);
        let chunk = ChunkInput::new(transition, Vec::new());
        EePrivateInput::new(Vec::new(), Vec::new(), vec![chunk])
    }

    /// Round-trip a `DaWitness` and `EePrivateInput` through rkyv so we
    /// can hand the archived forms to `verify_da_witness`.
    fn rkyv_witnesses(da: &DaWitness, ee: &EePrivateInput) -> (Vec<u8>, Vec<u8>) {
        let da_bytes = rkyv::to_bytes::<RkyvError>(da).unwrap().to_vec();
        let ee_bytes = rkyv::to_bytes::<RkyvError>(ee).unwrap().to_vec();
        (da_bytes, ee_bytes)
    }

    /// Helper that takes already-serialized rkyv bytes and returns
    /// archived references — avoids per-test boilerplate.
    fn archive<'a>(
        da_bytes: &'a [u8],
        ee_bytes: &'a [u8],
    ) -> (&'a ArchivedDaWitness, &'a ArchivedEePrivateInput) {
        let da = rkyv::access::<ArchivedDaWitness, RkyvError>(da_bytes).unwrap();
        let ee = rkyv::access::<ArchivedEePrivateInput, RkyvError>(ee_bytes).unwrap();
        (da, ee)
    }

    #[test]
    fn empty_da_witness_returns_none() {
        let ee = EePrivateInput::new(Vec::new(), Vec::new(), Vec::new());
        let (da_bytes, ee_bytes) = rkyv_witnesses(&DaWitness::empty(), &ee);
        let (da, ee_arch) = archive(&da_bytes, &ee_bytes);
        assert!(verify_da_witness(ee_arch, da).unwrap().is_none());
    }

    #[test]
    fn reassemble_and_bind_round_trips_blob() {
        let prev = Hash::from([0x11; 32]);
        let last = Hash::from([0x22; 32]);
        let blob = fake_da_blob(prev, last);
        let witness = build_da_witness_for_blob(&blob);
        let ee = build_ee_input_with_endpoints(prev, last);

        let (da_bytes, ee_bytes) = rkyv_witnesses(&witness, &ee);
        let (da, ee_arch) = archive(&da_bytes, &ee_bytes);

        let recovered = reassemble_and_bind_batch_id(ee_arch, da).unwrap();
        assert_eq!(recovered.batch_id.prev_block(), prev);
        assert_eq!(recovered.batch_id.last_block(), last);
    }

    #[test]
    fn reassemble_and_bind_rejects_mismatched_batch_id() {
        let blob = fake_da_blob(Hash::from([0x11; 32]), Hash::from([0x22; 32]));
        let witness = build_da_witness_for_blob(&blob);
        // Chunks claim a different batch endpoint than the blob carries.
        let ee = build_ee_input_with_endpoints(Hash::from([0x99; 32]), Hash::from([0xaa; 32]));

        let (da_bytes, ee_bytes) = rkyv_witnesses(&witness, &ee);
        let (da, ee_arch) = archive(&da_bytes, &ee_bytes);

        let err = reassemble_and_bind_batch_id(ee_arch, da).unwrap_err();
        assert!(matches!(err, DaVerificationError::BatchIdMismatch(_)));
    }

    #[test]
    fn reassemble_and_bind_rejects_garbage_envelope_payload() {
        // DA witness carries a single payload that isn't a valid encoded chunk.
        let reveal = RevealWitness::new(
            [0u8; 32],
            [0u8; 32],
            BitcoinMerkleProof::default(),
            vec![0xFF; 8],
        );
        let block = DaBlockWitness::new(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            BitcoinMerkleProof::default(),
            vec![reveal],
        );
        let witness = DaWitness::new([0xee; 32], vec![block]);
        let ee = build_ee_input_with_endpoints(Hash::from([0x11; 32]), Hash::from([0x22; 32]));

        let (da_bytes, ee_bytes) = rkyv_witnesses(&witness, &ee);
        let (da, ee_arch) = archive(&da_bytes, &ee_bytes);

        let err = reassemble_and_bind_batch_id(ee_arch, da).unwrap_err();
        assert!(matches!(err, DaVerificationError::Reassembly(_)));
    }

    mod ledger_refs {
        use strata_snark_acct_types::{AccumulatorClaim, LedgerRefs};

        use super::*;

        fn da_witness_with_tip(tip: [u8; 32]) -> DaWitness {
            // One block with no reveals — only the tip hash matters
            // for `bind_da_witness_to_ledger_refs`.
            let block = DaBlockWitness::new(
                Vec::new(),
                Vec::new(),
                Vec::new(),
                BitcoinMerkleProof::default(),
                Vec::new(),
            );
            DaWitness::new(tip, vec![block])
        }

        fn rkyv_da(witness: &DaWitness) -> Vec<u8> {
            rkyv::to_bytes::<RkyvError>(witness).unwrap().to_vec()
        }

        #[test]
        fn empty_witness_skips_check() {
            let bytes = rkyv_da(&DaWitness::empty());
            let archived = rkyv::access::<ArchivedDaWitness, RkyvError>(&bytes).unwrap();
            // Refs can be anything when witness is empty.
            let refs = LedgerRefs::new_empty();
            assert!(bind_da_witness_to_ledger_refs(archived, &refs).is_ok());
        }

        #[test]
        fn matching_tip_passes() {
            let tip = [0xab; 32];
            let bytes = rkyv_da(&da_witness_with_tip(tip));
            let archived = rkyv::access::<ArchivedDaWitness, RkyvError>(&bytes).unwrap();
            let refs = LedgerRefs::new(vec![
                AccumulatorClaim::new(100, [0x11; 32]),
                AccumulatorClaim::new(200, tip),
            ]);
            assert!(bind_da_witness_to_ledger_refs(archived, &refs).is_ok());
        }

        #[test]
        fn mismatched_tip_fails() {
            let bytes = rkyv_da(&da_witness_with_tip([0xab; 32]));
            let archived = rkyv::access::<ArchivedDaWitness, RkyvError>(&bytes).unwrap();
            let refs = LedgerRefs::new(vec![AccumulatorClaim::new(200, [0xcd; 32])]);
            let err = bind_da_witness_to_ledger_refs(archived, &refs).unwrap_err();
            assert!(matches!(err, DaVerificationError::LedgerTipMismatch { .. }));
        }

        #[test]
        fn empty_refs_with_da_present_fails() {
            let bytes = rkyv_da(&da_witness_with_tip([0xab; 32]));
            let archived = rkyv::access::<ArchivedDaWitness, RkyvError>(&bytes).unwrap();
            let refs = LedgerRefs::new_empty();
            let err = bind_da_witness_to_ledger_refs(archived, &refs).unwrap_err();
            assert!(matches!(err, DaVerificationError::LedgerRefsEmpty));
        }
    }

    /// End-to-end test: builds a synthetic Bitcoin block with a real
    /// coinbase + witness commitment + reveal txs that wrap our chunked-
    /// envelope payloads, threads through `verify_da_witness`, and
    /// asserts inclusion + reassembly + batch-id binding all pass.
    mod e2e {
        use core::iter;

        use bitcoin::{
            Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness,
            absolute::LockTime, consensus::serialize, script::Builder as ScriptBuilder,
            transaction::Version,
        };
        use strata_crypto::hash::sha256d;
        use strata_identifiers::Buf32;

        use super::*;

        fn dsha(data: &[u8]) -> [u8; 32] {
            let h: Buf32 = sha256d(data);
            *<Buf32 as AsRef<[u8; 32]>>::as_ref(&h)
        }

        /// Bitcoin Merkle root for a list of leaves with last-element
        /// duplication on odd levels, mirroring the host-side builder.
        fn merkle_root(leaves: &[[u8; 32]]) -> [u8; 32] {
            let mut cur = leaves.to_vec();
            while cur.len() > 1 {
                if cur.len() % 2 == 1 {
                    cur.push(*cur.last().unwrap());
                }
                cur = cur
                    .chunks(2)
                    .map(|p| {
                        let mut buf = [0u8; 64];
                        buf[..32].copy_from_slice(&p[0]);
                        buf[32..].copy_from_slice(&p[1]);
                        dsha(&buf)
                    })
                    .collect();
            }
            cur[0]
        }

        /// Generates an inclusion proof for `leaves[idx]` (same shape
        /// our host-side builder produces, kept inline to keep the test
        /// self-contained).
        fn inclusion_proof(leaves: &[[u8; 32]], idx: usize) -> BitcoinMerkleProof {
            let mut cur = leaves.to_vec();
            let mut cur_idx = idx;
            let mut siblings = Vec::new();
            while cur.len() > 1 {
                if cur.len() % 2 == 1 {
                    cur.push(*cur.last().unwrap());
                }
                let sib = cur_idx ^ 1;
                siblings.push(cur[sib]);
                cur = cur
                    .chunks(2)
                    .map(|p| {
                        let mut buf = [0u8; 64];
                        buf[..32].copy_from_slice(&p[0]);
                        buf[32..].copy_from_slice(&p[1]);
                        dsha(&buf)
                    })
                    .collect();
                cur_idx >>= 1;
            }
            BitcoinMerkleProof::new(siblings, idx as u32)
        }

        fn make_reveal_tx_carrying_payload(payload: &[u8]) -> Transaction {
            // Witness must contain the chunked-envelope script as its
            // tapscript leaf. Build a minimal script:
            //   <pubkey> CHECKSIG OP_FALSE OP_IF <payload> OP_ENDIF
            let mut script_bytes = Vec::new();
            // pubkey push (33 bytes of zeros + checksig is fine for
            // structure-only verification).
            script_bytes.push(33);
            script_bytes.extend(iter::repeat_n(0u8, 33));
            script_bytes.push(0xac); // OP_CHECKSIG
            script_bytes.push(0x00); // OP_FALSE
            script_bytes.push(0x63); // OP_IF
            // Push payload in <=520-byte chunks.
            for piece in payload.chunks(520) {
                if piece.len() < 0x4c {
                    script_bytes.push(piece.len() as u8);
                } else {
                    script_bytes.push(0x4c);
                    script_bytes.push(piece.len() as u8);
                }
                script_bytes.extend_from_slice(piece);
            }
            script_bytes.push(0x68); // OP_ENDIF
            let leaf_script = ScriptBuf::from_bytes(script_bytes);

            let mut witness = Witness::new();
            witness.push([0u8; 64]); // dummy signature
            witness.push(leaf_script.as_bytes());
            witness.push([0u8; 33]); // dummy control block

            Transaction {
                version: Version::TWO,
                lock_time: LockTime::ZERO,
                input: vec![TxIn {
                    previous_output: OutPoint::null(),
                    script_sig: ScriptBuf::new(),
                    sequence: Sequence::MAX,
                    witness,
                }],
                output: vec![TxOut {
                    value: Amount::from_sat(0),
                    script_pubkey: ScriptBuf::new(),
                }],
            }
        }

        fn make_coinbase_with_commitment(witness_root: [u8; 32]) -> Transaction {
            let reserved = [0u8; 32];
            let mut buf = [0u8; 64];
            buf[..32].copy_from_slice(&witness_root);
            buf[32..].copy_from_slice(&reserved);
            let commitment = dsha(&buf);

            let mut script = vec![0x6a, 0x24, 0xaa, 0x21, 0xa9, 0xed];
            script.extend_from_slice(&commitment);
            let commitment_script = ScriptBuf::from_bytes(script);

            let mut witness = Witness::new();
            witness.push(reserved);

            Transaction {
                version: Version::TWO,
                lock_time: LockTime::ZERO,
                input: vec![TxIn {
                    previous_output: OutPoint::null(),
                    script_sig: ScriptBuilder::new().push_int(0).into_script(),
                    sequence: Sequence::MAX,
                    witness,
                }],
                output: vec![
                    TxOut {
                        value: Amount::from_sat(50_0000_0000),
                        script_pubkey: ScriptBuf::new(),
                    },
                    TxOut {
                        value: Amount::from_sat(0),
                        script_pubkey: commitment_script,
                    },
                ],
            }
        }

        fn make_header(prev: [u8; 32], merkle_root: [u8; 32]) -> Vec<u8> {
            let mut h = Vec::with_capacity(80);
            h.extend_from_slice(&1u32.to_le_bytes());
            h.extend_from_slice(&prev);
            h.extend_from_slice(&merkle_root);
            h.extend_from_slice(&1_700_000_000u32.to_le_bytes());
            h.extend_from_slice(&0x1d00ffffu32.to_le_bytes());
            h.extend_from_slice(&0u32.to_le_bytes());
            h
        }

        #[test]
        fn full_pipeline_pass_with_synthetic_block() {
            // 1. Build the published DaBlob.
            let prev = Hash::from([0x11; 32]);
            let last = Hash::from([0x22; 32]);
            let blob = fake_da_blob(prev, last);

            // 2. Encode the blob into chunks; one reveal per chunk.
            let chunks = prepare_da_chunks(&blob).expect("encode chunks");
            assert!(!chunks.is_empty());

            // 3. Build reveal txs carrying each chunk in their witness envelope.
            let reveal_txs: Vec<Transaction> = chunks
                .iter()
                .map(|c| make_reveal_tx_carrying_payload(c))
                .collect();

            // 4. Compute wtxids; build witness Merkle root with coinbase leaf zeroed.
            let wtxid_leaves: Vec<[u8; 32]> = iter::once([0u8; 32])
                .chain(reveal_txs.iter().map(|t| t.compute_wtxid().to_byte_array()))
                .collect();
            let witness_root = merkle_root(&wtxid_leaves);

            // 5. Build coinbase committing to witness_root, then full block tx list.
            let coinbase = make_coinbase_with_commitment(witness_root);
            let mut all_txs = vec![coinbase.clone()];
            all_txs.extend(reveal_txs.iter().cloned());

            // 6. Compute txid Merkle root (header.merkle_root).
            let txid_leaves: Vec<[u8; 32]> = all_txs
                .iter()
                .map(|t| t.compute_txid().to_byte_array())
                .collect();
            let header_merkle_root = merkle_root(&txid_leaves);

            // 7. Build header with that merkle_root; tip = block_hash.
            let raw_header = make_header([0u8; 32], header_merkle_root);
            let tip_hash = dsha(&raw_header);

            // 8. Build per-reveal RevealWitness with wtxid Merkle proofs.
            let coinbase_proof = inclusion_proof(&txid_leaves, 0);
            let reveal_witnesses: Vec<RevealWitness> = reveal_txs
                .iter()
                .enumerate()
                .map(|(i, tx)| {
                    let proof = inclusion_proof(&wtxid_leaves, i + 1);
                    RevealWitness::new(
                        tx.compute_txid().to_byte_array(),
                        tx.compute_wtxid().to_byte_array(),
                        proof,
                        chunks[i].clone(),
                    )
                })
                .collect();

            let block_witness = DaBlockWitness::new(
                raw_header,
                Vec::new(), // tip == this block, chain empty
                serialize(&coinbase),
                coinbase_proof,
                reveal_witnesses,
            );
            let witness = DaWitness::new(tip_hash, vec![block_witness]);
            let ee = build_ee_input_with_endpoints(prev, last);

            // 9. Run the full pipeline: verify_da_witness + tip-binding.
            let (da_bytes, ee_bytes) = rkyv_witnesses(&witness, &ee);
            let (da, ee_arch) = archive(&da_bytes, &ee_bytes);
            let recovered = verify_da_witness(ee_arch, da)
                .expect("e2e verify must succeed")
                .expect("blob present");
            assert_eq!(recovered.batch_id.prev_block(), prev);
            assert_eq!(recovered.batch_id.last_block(), last);

            // 10. Tip binding. Host-built LedgerRefs anchor to the same tip as the DA witness.
            use strata_snark_acct_types::{AccumulatorClaim, LedgerRefs};
            let ledger_refs = LedgerRefs::new(vec![AccumulatorClaim::new(42, tip_hash)]);
            bind_da_witness_to_ledger_refs(da, &ledger_refs).expect("matching tip must bind");
        }
    }
}
