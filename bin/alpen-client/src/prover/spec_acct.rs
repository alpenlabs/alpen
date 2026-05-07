//! Acct (outer/update) [`ProofSpec`].
//!
//! Task = [`BatchId`] (newtype-wrapped). Program = [`EeAcctProgram`];
//! `fetch_input` reads chunk receipts (from the shared paas
//! `ReceiptStore`) and the prior batch's end-state, then assembles
//! `ee_acct_runtime::PrivateInput` + `snark_acct_runtime::PrivateInput`.
//!
//! DA witnesses are built from `BatchStatus::DaComplete { da }` and
//! anchored to a sequencer-selected L1 tip
//! (`l1_reorg_safe_depth`-confirmation horizon below the chain tip,
//! falling back to the highest DA block when DA is freshly published).

use std::{collections::HashMap, fmt, sync::Arc};

use alloy_primitives::B256;
use alpen_ee_common::{
    BatchId, BatchStatus, BatchStorage, ExecBlockStorage, L1DaBlockRef, Storage,
};
use alpen_ee_database::{BroadcastDbOps, EeNodeStorage};
use async_trait::async_trait;
use bitcoin::{
    consensus::{deserialize, serialize},
    hashes::Hash as _,
    Transaction,
};
use bitcoind_async_client::{traits::Reader, Client as BtcClient};
use rsp_primitives::genesis::Genesis;
use ssz::{Decode, Encode as _};
use strata_acct_types::Hash;
use strata_btcio::writer::chunked_envelope::extract_chunk_envelope_payload;
use strata_codec::encode_to_vec;
use strata_ee_acct_runtime::{
    ChunkInput, DaBlockWitness, DaWitness, EePrivateInput, RevealWitness,
};
use strata_ee_acct_types::UpdateExtraData;
use strata_ee_chain_types::ChunkTransition;
use strata_paas::{ProofSpec, ProverError as PaasError, ProverResult, ReceiptStore};
use strata_primitives::{buf::Buf32, l1::L1BlockIdBitcoinExt};
use strata_proofimpl_alpen_acct::{EeAcctProgram, EeAcctProofInput};
use strata_snark_acct_runtime::{Coinput, IInnerState, PrivateInput as UpdatePrivateInput};
use strata_snark_acct_types::{
    AccumulatorClaim, LedgerRefs, OutputMessage, OutputTransfer, ProofState, UpdateOutputs,
    UpdateProofPubParams,
};
use tokio::task;

use super::{
    da_witness_build::{build_coinbase_inclusion_proof, build_wtxid_inclusion_proof},
    ChunkTask, RangeWitnessFn,
};

/// Batch-id-shaped task identifier for paas. Newtype over [`BatchId`]
/// for the same reasons [`super::ChunkTask`] wraps `ChunkId`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct BatchTask(pub BatchId);

impl fmt::Display for BatchTask {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Single-byte kind tag for [`BatchTask`] encoding; see the matching
/// `CHUNK_TASK_TAG` on `ChunkTask` for why the shared prover-task tree
/// needs a discriminator.
pub(crate) const BATCH_TASK_TAG: u8 = b'a';

/// Tag byte + the underlying `BatchId`'s bytes.
const BATCH_TASK_BYTES: usize = 1 + size_of::<BatchId>();

impl From<BatchTask> for Vec<u8> {
    fn from(task: BatchTask) -> Self {
        let mut buf = Vec::with_capacity(BATCH_TASK_BYTES);
        buf.push(BATCH_TASK_TAG);
        let prev: [u8; 32] = task.0.prev_block().into();
        let last: [u8; 32] = task.0.last_block().into();
        buf.extend_from_slice(&prev);
        buf.extend_from_slice(&last);
        buf
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum BatchTaskDecodeError {
    #[error("invalid BatchTask byte length: expected {BATCH_TASK_BYTES}, got {0}")]
    InvalidLength(usize),
    #[error("invalid BatchTask tag byte: expected 0x{BATCH_TASK_TAG:02x}, got 0x{0:02x}")]
    InvalidTag(u8),
}

impl TryFrom<Vec<u8>> for BatchTask {
    type Error = BatchTaskDecodeError;

    fn try_from(bytes: Vec<u8>) -> Result<Self, Self::Error> {
        if bytes.len() != BATCH_TASK_BYTES {
            return Err(BatchTaskDecodeError::InvalidLength(bytes.len()));
        }
        if bytes[0] != BATCH_TASK_TAG {
            return Err(BatchTaskDecodeError::InvalidTag(bytes[0]));
        }
        let mut prev = [0u8; 32];
        let mut last = [0u8; 32];
        prev.copy_from_slice(&bytes[1..33]);
        last.copy_from_slice(&bytes[33..]);
        Ok(BatchTask(BatchId::from_parts(
            Hash::from(prev),
            Hash::from(last),
        )))
    }
}

/// Outer-proof specification.
///
/// Holds the shared paas `ReceiptStore` (chunk receipts the chunk
/// prover wrote), `Arc<dyn BatchStorage>` for batch metadata,
/// `Arc<EeNodeStorage>` for `ExecBlockRecord` + `EeAccountState`
/// reads, and `Arc<EeBatchProofDbManager>` so the struct can be
/// shared with the receipt hook (which writes outer proofs there).
#[derive(Clone)]
pub(crate) struct AcctSpec {
    chunk_receipts: Arc<dyn ReceiptStore>,
    batch_storage: Arc<dyn BatchStorage>,
    storage: Arc<EeNodeStorage>,
    broadcast_ops: Arc<BroadcastDbOps>,
    btc_client: Arc<BtcClient>,
    /// Same closure [`super::ChunkSpec`] uses; produces a sparse
    /// [`alpen_reth_witness::RangeWitnessData`] covering the union of
    /// all blocks' read sets in a range. We call it on the batch's
    /// `(first_block, last_block)` so the guest can apply the
    /// reassembled state diff against the same MPT shape the chunk
    /// pipeline witnesses.
    range_witness_fn: Arc<RangeWitnessFn>,
    genesis: Genesis,
    /// Number of L1 confirmations to keep on top of the proof's anchor
    /// tip. The acct proof's `l1_block_hash` is chosen `depth` blocks
    /// below the chain tip when possible, so OL canonicality checks the
    /// proof against an L1 reference that's already past the local
    /// reorg-safe horizon. Falls back to the highest DA block when the
    /// chain tip isn't deep enough yet (DA freshly published).
    l1_reorg_safe_depth: u32,
}

impl AcctSpec {
    #[expect(
        clippy::too_many_arguments,
        reason = "AcctSpec is wired in once at startup; \
                  collapsing into a builder/config struct would just \
                  add indirection without locality benefit"
    )]
    pub(crate) fn new(
        chunk_receipts: Arc<dyn ReceiptStore>,
        batch_storage: Arc<dyn BatchStorage>,
        storage: Arc<EeNodeStorage>,
        broadcast_ops: Arc<BroadcastDbOps>,
        btc_client: Arc<BtcClient>,
        range_witness_fn: Arc<RangeWitnessFn>,
        genesis: Genesis,
        l1_reorg_safe_depth: u32,
    ) -> Self {
        Self {
            chunk_receipts,
            batch_storage,
            storage,
            broadcast_ops,
            btc_client,
            range_witness_fn,
            genesis,
            l1_reorg_safe_depth,
        }
    }
}

#[async_trait]
impl ProofSpec for AcctSpec {
    type Task = BatchTask;
    type Program = EeAcctProgram;

    async fn fetch_input(&self, task: &Self::Task) -> ProverResult<EeAcctProofInput> {
        let batch_id = task.0;

        // 1. Chunk inputs: per-chunk transitions + their proofs, in order.
        let chunks: Vec<ChunkInput> =
            collect_chunk_inputs_for_batch(&*self.batch_storage, &*self.chunk_receipts, batch_id)
                .await?;
        if chunks.is_empty() {
            return Err(PaasError::PermanentFailure(format!(
                "batch {batch_id} has no chunks"
            )));
        }

        // 2. Batch metadata. The first block is the one immediately after `prev_block`; we resolve
        //    it via the FIRST chunk's parent_blkid.
        let (batch, status) = self
            .batch_storage
            .get_batch_by_id(batch_id)
            .await
            .map_err(|e| PaasError::Storage(format!("get_batch_by_id({batch_id}): {e}")))?
            .ok_or_else(|| {
                PaasError::PermanentFailure(format!("batch {batch_id} not in storage"))
            })?;

        // 3. Previous EE account state.
        //
        //    We read the latest OL-accepted EeAccountState and verify its
        //    tip matches the first chunk's parent. This works because only
        //    one batch is proved at a time — the batch lifecycle
        //    (BatchLifecycleState::proof_pending) advances the frontier
        //    one batch at a time, so by the time we're here the previous
        //    batch has already landed on OL and best_ee_account_state
        //    reflects it.
        //
        //    If batch pipelining is ever added (multiple batches proving
        //    concurrently), this call would return a stale pre-state and
        //    the check below would fire as TransientFailure. The fix
        //    would be a local EeAccountState projection that walks
        //    sequenced-but-unposted batches forward from the OL tip.
        let acct_at_epoch = self
            .storage
            .best_ee_account_state()
            .await
            .map_err(|e| PaasError::Storage(format!("best_ee_account_state: {e}")))?;
        let acct_at_epoch = acct_at_epoch.ok_or_else(|| {
            PaasError::TransientFailure(
                "no EE account state available yet (genesis not loaded?)".to_string(),
            )
        })?;
        let pre_ee_state = acct_at_epoch.ee_state().clone();

        let first_chunk = chunks.first().ok_or_else(|| {
            PaasError::PermanentFailure("first chunk missing after non-empty check".to_string())
        })?;
        let first_transition = decode_chunk_transition(first_chunk)?;
        if first_transition.parent_exec_blkid() != pre_ee_state.last_exec_blkid() {
            return Err(PaasError::TransientFailure(format!(
                "EE pre-state mismatch for batch {batch_id}: \
                 first chunk parent={:?}, ee_state.last_exec_blkid={:?}",
                first_transition.parent_exec_blkid(),
                pre_ee_state.last_exec_blkid(),
            )));
        }

        // 4. ee_acct private input.
        //
        //    Pre-state for the in-guest state-diff consistency check:
        //    extract a sparse `EvmPartialState` covering the union of
        //    the batch's blocks' read sets via `RangeWitnessExtractor`
        //    (same closure `ChunkSpec` uses for per-chunk witnesses;
        //    here we span the full batch instead of a single chunk).
        //
        //    `raw_prev_header` is unused by today's acct guest (the
        //    chunk pubvals already pin parent/tip via `_exec_blkid`),
        //    so we leave it empty — populating it would only matter if
        //    we later add a header-chain check independent of chunk
        //    pubvals.
        let batch_block_hashes: Vec<Hash> = batch.blocks_iter().collect();
        let first_block_hash = B256::from(
            batch_block_hashes
                .first()
                .copied()
                .expect("batch has chunks ⇒ at least one block")
                .0,
        );
        let last_block_hash_eth = B256::from(batch.last_block().0);
        let range_fn = self.range_witness_fn.clone();
        let range_data =
            task::spawn_blocking(move || (range_fn)(first_block_hash, last_block_hash_eth))
                .await
                .map_err(|e| PaasError::TransientFailure(format!("witness extraction join: {e}")))?
                .map_err(|e| PaasError::TransientFailure(format!("witness extraction: {e}")))?;
        let raw_partial_pre_state = range_data.raw_partial_pre_state;

        let ee_private_input =
            EePrivateInput::new(Vec::new(), raw_partial_pre_state, chunks.clone());

        // 5. Build UpdateProofPubParams from ExecBlockRecords.
        //
        //    Mirrors the submission-side `update_builder.rs` construction:
        //    read each block's ExecBlockRecord to derive processed_inputs,
        //    messages, outputs, next_inbox_msg_idx, and new_tip_blkid.
        //    This is the authoritative source (same data the update
        //    submitter reads), so proof-input and submission agree.
        let block_hashes: Vec<Hash> = batch.blocks_iter().collect();
        let mut processed_inputs: u32 = 0;
        let mut messages = Vec::new();
        let mut update_outputs = UpdateOutputs::new_empty();
        for block_hash in &block_hashes {
            let record = self
                .storage
                .get_exec_block(*block_hash)
                .await
                .map_err(|e| PaasError::Storage(format!("get_exec_block({block_hash:?}): {e}")))?
                .ok_or_else(|| {
                    PaasError::TransientFailure(format!(
                        "ExecBlockRecord missing for {block_hash:?} in batch {batch_id}"
                    ))
                })?;

            let (package, _account_state, mut block_messages) = record.into_parts();
            processed_inputs += package.inputs().total_inputs() as u32;
            messages.append(&mut block_messages);
            update_outputs
                .try_extend_transfers(
                    package
                        .outputs()
                        .output_transfers()
                        .iter()
                        .map(|t| OutputTransfer::new(t.dest(), t.value())),
                )
                .map_err(|_| {
                    PaasError::PermanentFailure("UpdateOutputs transfers overflow".to_string())
                })?;
            update_outputs
                .try_extend_messages(
                    package
                        .outputs()
                        .output_messages()
                        .iter()
                        .map(|m| OutputMessage::new(m.dest(), m.payload().clone())),
                )
                .map_err(|_| {
                    PaasError::PermanentFailure("UpdateOutputs messages overflow".to_string())
                })?;
        }

        // Last block gives us post-batch metadata.
        let last_block_hash = batch.last_block();
        let last_record = self
            .storage
            .get_exec_block(last_block_hash)
            .await
            .map_err(|e| PaasError::Storage(format!("get_exec_block({last_block_hash:?}): {e}")))?
            .ok_or_else(|| {
                PaasError::PermanentFailure(format!(
                    "last block record missing for batch {batch_id}"
                ))
            })?;
        let new_tip_blkid = last_record.package().exec_blkid();
        let new_inbox_idx = last_record.next_inbox_msg_idx();
        let pre_inbox_idx = new_inbox_idx - messages.len() as u64;

        // Derive pre/post state roots. We advance `pre_ee_state` the same
        // way the EE program's `pre_finalize_state` does (set tip blkid)
        // so the proof guest's computation from `raw_pre_state` arrives at
        // the same post-root.
        let mut post_ee_state = pre_ee_state.clone();
        post_ee_state.set_last_exec_blkid(new_tip_blkid);

        let cur_state = ProofState::new(pre_ee_state.compute_state_root(), pre_inbox_idx);
        let new_state = ProofState::new(post_ee_state.compute_state_root(), new_inbox_idx);

        let extra_data = UpdateExtraData::new(new_tip_blkid, processed_inputs, 0);
        let extra_data_bytes = encode_to_vec(&extra_data)
            .map_err(|e| PaasError::PermanentFailure(format!("encode extra data: {e}")))?;

        // Pick the L1 anchor: deepest height we can reach `≥` than the
        // highest DA block while sitting `l1_reorg_safe_depth` confirmations
        // below the chain tip. Same anchor flows into the DA witness's
        // `l1_block_hash` and the highest-idx `LedgerRefs` claim — the
        // guest binds them together (`bind_da_witness_to_ledger_refs`).
        let safe_tip = select_safe_tip(
            da_refs_from_status(&status),
            self.l1_reorg_safe_depth,
            &*self.btc_client,
        )
        .await?;

        // OL handshake: one AccumulatorClaim per L1 block touched by DA,
        // plus the safe-tip block when it sits above all DA blocks.
        // Ordered ascending by height. OL re-checks each `idx ↦ entry_hash`
        // against its own L1 Header MMR; the highest-idx entry's
        // `entry_hash` is the tip the proof anchors to (matches
        // `da_witness.l1_block_hash` set in `build_da_witness`).
        let ledger_refs = build_ledger_refs(da_refs_from_status(&status), safe_tip);

        let pub_params = UpdateProofPubParams::new(
            cur_state,
            new_state,
            messages,
            ledger_refs,
            update_outputs,
            extra_data_bytes,
        );

        // Coinputs: one empty coinput per message (EE program requires
        // empty coinputs — see verify_coinput in ee_program.rs).
        let coinputs = pub_params
            .message_inputs()
            .iter()
            .map(|_| Coinput::new(Vec::new()))
            .collect();

        let update_private_input =
            UpdatePrivateInput::new(pub_params, pre_ee_state.as_ssz_bytes(), coinputs);

        // DA witness: full reveal-tx inclusion proofs anchored to
        // `safe_tip`. Envelope payloads come from the broadcast DB;
        // headers, coinbase, and Merkle paths come from the L1 client.
        // Each block's `raw_header_chain_to_tip` bridges from that
        // block up to `safe_tip` via prev_blockhash chaining, so a
        // header chain of 0..N hops is uniformly handled in-guest.
        let da_witness = build_da_witness(
            da_refs_from_status(&status),
            safe_tip,
            &self.broadcast_ops,
            &*self.btc_client,
        )
        .await?;

        Ok(EeAcctProofInput {
            genesis: self.genesis.clone(),
            ee_private_input,
            update_private_input,
            da_witness,
        })
    }
}

/// Extracts the DA L1 block refs from a [`BatchStatus`] if it carries
/// any. Returns `&[]` for variants without DA data (Genesis, Sealed,
/// DaPending) — `fetch_input` should still produce a structurally
/// valid (but empty) [`DaWitness`] in those cases so the guest input
/// shape stays uniform.
fn da_refs_from_status(status: &BatchStatus) -> &[L1DaBlockRef] {
    match status {
        BatchStatus::DaComplete { da }
        | BatchStatus::ProofPending { da }
        | BatchStatus::ProofReady { da, .. } => da.as_slice(),
        BatchStatus::Genesis | BatchStatus::Sealed | BatchStatus::DaPending { .. } => &[],
    }
}

/// L1 anchor the acct proof binds its DA inclusion checks to.
///
/// `height` is at least the highest DA-touched block's height; when
/// the chain has advanced enough above the DA, it sits
/// `l1_reorg_safe_depth` confirmations below the chain tip — the same
/// horizon `BroadcasterProcessor` uses to mark txs finalized.
#[derive(Clone, Copy, Debug)]
struct SafeTip {
    height: u64,
    hash: [u8; 32],
}

/// Subset of [`Reader`] that [`select_safe_tip`] depends on.
///
/// Lets the unit tests stub two methods instead of the ten the full
/// `Reader` trait expects. Blanket-impl below covers the production
/// path where `select_safe_tip` runs on the real `BtcClient`.
trait SafeTipReader {
    async fn chain_tip_height(&self) -> ProverResult<u64>;
    async fn block_hash_at(&self, height: u64) -> ProverResult<[u8; 32]>;
}

impl<R: Reader + Sync> SafeTipReader for R {
    async fn chain_tip_height(&self) -> ProverResult<u64> {
        self.get_block_count()
            .await
            .map_err(|e| PaasError::Storage(format!("get_block_count: {e}")))
    }

    async fn block_hash_at(&self, height: u64) -> ProverResult<[u8; 32]> {
        let hash = self
            .get_block_hash(height)
            .await
            .map_err(|e| PaasError::Storage(format!("get_block_hash({height}): {e}")))?;
        Ok(hash.to_byte_array())
    }
}

/// Picks the L1 anchor for the proof.
///
/// Returns the deepest tip such that `tip.height ≥ max_da_height` and
/// `tip.height + l1_reorg_safe_depth ≤ chain_tip_height`. Falls back
/// to the highest DA-touched block when DA is too fresh for the safe
/// depth to apply on top — in that regime OL canonicality enforces
/// the depth on its end and the proof still anchors correctly.
async fn select_safe_tip(
    da_refs: &[L1DaBlockRef],
    l1_reorg_safe_depth: u32,
    btc: &impl SafeTipReader,
) -> ProverResult<Option<SafeTip>> {
    if da_refs.is_empty() {
        return Ok(None);
    }

    let max_da_height = da_refs
        .iter()
        .map(|r| r.block.height() as u64)
        .max()
        .expect("non-empty per check above");

    let chain_tip_height = btc.chain_tip_height().await?;
    let depth_safe_height = chain_tip_height.saturating_sub(l1_reorg_safe_depth as u64);
    let height = max_da_height.max(depth_safe_height);

    // Reuse the DA ref's hash when the safe tip lands on a DA block —
    // saves one RPC round trip in the freshly-published case.
    let hash = if height == max_da_height {
        let tip_ref = da_refs
            .iter()
            .find(|r| r.block.height() as u64 == max_da_height)
            .expect("max_da_height was derived from da_refs");
        *AsRef::<[u8; 32]>::as_ref(tip_ref.block.blkid())
    } else {
        btc.block_hash_at(height).await?
    };

    Ok(Some(SafeTip { height, hash }))
}

/// Builds a full [`DaWitness`] for the batch under proof, anchored to
/// `safe_tip`.
///
/// For each [`L1DaBlockRef`]: fetches the L1 block, serializes its
/// header and coinbase, builds a coinbase-txid → header.merkle_root
/// Merkle proof, and per reveal builds a wtxid → witness-root proof
/// plus lifts the envelope payload from the local broadcast DB.
/// Header-chain bridging fetches headers for the heights between each
/// DA block and the safe tip — `raw_header_chain_to_tip` is empty for
/// a block that already sits at the tip, otherwise carries the
/// successor headers up to and including `safe_tip`.
async fn build_da_witness(
    da_refs: &[L1DaBlockRef],
    safe_tip: Option<SafeTip>,
    broadcast_ops: &BroadcastDbOps,
    btc: &impl Reader,
) -> ProverResult<DaWitness> {
    if da_refs.is_empty() {
        return Ok(DaWitness::empty());
    }
    let safe_tip = safe_tip.expect("non-empty da_refs imply Some(safe_tip)");

    let mut sorted: Vec<&L1DaBlockRef> = da_refs.iter().collect();
    sorted.sort_by_key(|r| r.block.height());

    let tip_height = safe_tip.height;
    let l1_block_hash = safe_tip.hash;

    // Pre-fetch headers for every height in [min_da_height + 1, tip].
    // Each DA block's `raw_header_chain_to_tip` slice is then a window
    // of this map — bridges the block up to the safe tip via
    // prev_blockhash chaining. Using height-indexed fetch keeps us on
    // the canonical chain the L1 client sees.
    let min_height = sorted[0].block.height() as u64;
    let mut header_at_height: HashMap<u64, Vec<u8>> = HashMap::new();
    for h in (min_height + 1)..=tip_height {
        let header = btc
            .get_block_header_at(h)
            .await
            .map_err(|e| PaasError::Storage(format!("get_block_header_at({h}): {e}")))?;
        header_at_height.insert(h, serialize(&header));
    }

    let mut blocks = Vec::with_capacity(sorted.len());
    for r in sorted {
        let height = r.block.height() as u64;
        let block_hash = r.block.blkid().to_block_hash();
        let block = btc
            .get_block(&block_hash)
            .await
            .map_err(|e| PaasError::Storage(format!("get_block({block_hash}): {e}")))?;

        if block.txdata.is_empty() {
            return Err(PaasError::PermanentFailure(format!(
                "L1 block {block_hash} has empty txdata"
            )));
        }
        let coinbase_proof = build_coinbase_inclusion_proof(&block.txdata);
        let raw_coinbase_tx = serialize(&block.txdata[0]);
        let raw_header = serialize(&block.header);

        let raw_header_chain_to_tip: Vec<Vec<u8>> = ((height + 1)..=tip_height)
            .map(|h| {
                header_at_height
                    .get(&h)
                    .cloned()
                    .expect("pre-fetched above")
            })
            .collect();

        let mut reveals = Vec::with_capacity(r.txns.len());
        for (txid, wtxid) in &r.txns {
            let pos = block
                .txdata
                .iter()
                .position(|t| t.compute_txid().to_byte_array() == txid.to_byte_array())
                .ok_or_else(|| {
                    PaasError::PermanentFailure(format!(
                        "reveal txid {txid} not found in L1 block {block_hash}"
                    ))
                })?;
            let wtxid_proof = build_wtxid_inclusion_proof(&block.txdata, pos);
            let envelope_payload = lift_reveal_envelope(broadcast_ops, txid).await?;
            reveals.push(RevealWitness::new(
                txid.to_byte_array(),
                wtxid.to_byte_array(),
                wtxid_proof,
                envelope_payload,
            ));
        }

        blocks.push(DaBlockWitness::new(
            raw_header,
            raw_header_chain_to_tip,
            raw_coinbase_tx,
            coinbase_proof,
            reveals,
        ));
    }

    Ok(DaWitness::new(l1_block_hash, blocks))
}

/// Builds [`LedgerRefs`] for the OL — one [`AccumulatorClaim`] per
/// [`L1DaBlockRef`], plus one for the safe-tip block when it sits
/// strictly above all DA blocks. `idx` = block height,
/// `entry_hash` = block hash. Sorted ascending by height.
///
/// On the OL side, each claim is re-verified against the L1 Header MMR
/// (height-indexed) when processing the resulting `EEUpdate`. The
/// safe-tip claim is what the OL canonicality + reorg-depth check
/// applies to; the in-between DA-block claims attest "DA was published
/// in these L1 blocks; you (OL) decide canonicality."
///
/// Because the highest-idx claim's `entry_hash` must equal
/// `da_witness.l1_block_hash` (verified in-guest by
/// `bind_da_witness_to_ledger_refs`), the safe-tip claim is the one
/// that anchors the whole proof when present.
fn build_ledger_refs(da_refs: &[L1DaBlockRef], safe_tip: Option<SafeTip>) -> LedgerRefs {
    let mut sorted: Vec<&L1DaBlockRef> = da_refs.iter().collect();
    sorted.sort_by_key(|r| r.block.height());
    let max_da_height = sorted.last().map(|r| r.block.height() as u64);
    let mut claims: Vec<AccumulatorClaim> = sorted
        .into_iter()
        .map(|r| {
            let height = r.block.height() as u64;
            let hash: [u8; 32] = *AsRef::<[u8; 32]>::as_ref(r.block.blkid());
            AccumulatorClaim::new(height, hash)
        })
        .collect();
    if let (Some(tip), Some(max_da)) = (safe_tip, max_da_height) {
        if tip.height > max_da {
            claims.push(AccumulatorClaim::new(tip.height, tip.hash));
        }
    }
    LedgerRefs::new(claims)
}

/// Fetches a reveal tx body from the broadcast DB and extracts the
/// chunked-envelope payload bytes that `decode_da_chunk` consumes.
///
/// Returns [`PaasError::TransientFailure`] if the broadcast entry is
/// not yet present (still being indexed) and [`PaasError::PermanentFailure`]
/// if the cached bytes fail to deserialize or the witness layout is
/// malformed (data corruption).
async fn lift_reveal_envelope(
    broadcast_ops: &BroadcastDbOps,
    txid: &bitcoin::Txid,
) -> ProverResult<Vec<u8>> {
    let txid_buf: Buf32 = txid.to_byte_array().into();
    let entry = broadcast_ops
        .get_tx_entry_by_id_async(txid_buf)
        .await
        .map_err(|e| PaasError::Storage(format!("get_tx_entry_by_id({txid}): {e}")))?
        .ok_or_else(|| {
            PaasError::TransientFailure(format!(
                "broadcast entry for reveal txid {txid} not yet present"
            ))
        })?;
    let tx: Transaction = deserialize(entry.tx_raw())
        .map_err(|e| PaasError::PermanentFailure(format!("deserialize reveal {txid}: {e}")))?;
    extract_chunk_envelope_payload(&tx)
        .map_err(|e| PaasError::PermanentFailure(format!("extract envelope for {txid}: {e}")))
}

/// Decodes a `ChunkInput`'s transition bytes. `PermanentFailure` on malformed.
fn decode_chunk_transition(ci: &ChunkInput) -> ProverResult<ChunkTransition> {
    ci.try_decode_chunk_transition()
        .map_err(|e| PaasError::PermanentFailure(format!("decode chunk transition: {e:?}")))
}

/// Collect [`ChunkInput`]s for a batch by reading per-chunk receipts from
/// the shared paas `ReceiptStore` (the chunk prover writes them after
/// proving).
///
/// Returns `TransientFailure` if any chunk's receipt is not yet present
/// (paas will retry on tick); returns `PermanentFailure` if a stored
/// receipt fails to decode as a [`ChunkTransition`] (data corruption).
async fn collect_chunk_inputs_for_batch(
    batch_storage: &dyn BatchStorage,
    chunk_receipts: &dyn ReceiptStore,
    batch_id: BatchId,
) -> ProverResult<Vec<ChunkInput>> {
    let chunk_ids = batch_storage
        .get_batch_chunks(batch_id)
        .await
        .map_err(|e| PaasError::Storage(format!("get_batch_chunks({batch_id}): {e}")))?
        .ok_or_else(|| {
            PaasError::PermanentFailure(format!("no chunks set for batch {batch_id}"))
        })?;

    let mut chunks = Vec::with_capacity(chunk_ids.len());
    for chunk_id in chunk_ids {
        let key: Vec<u8> = ChunkTask(chunk_id).into();
        let receipt = chunk_receipts.get(&key)?.ok_or_else(|| {
            PaasError::TransientFailure(format!(
                "chunk receipt missing for {chunk_id:?} (batch {batch_id})"
            ))
        })?;
        let pubvals = receipt.receipt().public_values().as_bytes();
        let transition = ChunkTransition::from_ssz_bytes(pubvals).map_err(|e| {
            PaasError::PermanentFailure(format!("decode ChunkTransition for {chunk_id:?}: {e:?}"))
        })?;
        let proof_bytes = receipt.receipt().proof().as_bytes().to_vec();
        chunks.push(ChunkInput::new(transition, proof_bytes));
    }

    Ok(chunks)
}

#[cfg(test)]
mod tests {
    use bitcoin::{Txid, Wtxid};
    use strata_identifiers::{L1BlockCommitment, L1BlockId};
    use strata_primitives::buf::Buf32;

    use super::*;

    fn ref_with(height: u32, blkid_byte: u8, txns: &[(u8, u8)]) -> L1DaBlockRef {
        let blkid = L1BlockId::from(Buf32::from([blkid_byte; 32]));
        let block = L1BlockCommitment::new(height, blkid);
        let txns = txns
            .iter()
            .map(|(t, w)| {
                (
                    Txid::from_byte_array([*t; 32]),
                    Wtxid::from_byte_array([*w; 32]),
                )
            })
            .collect();
        L1DaBlockRef::new(block, txns)
    }

    #[test]
    fn da_refs_from_status_returns_da_for_proof_pending() {
        let refs = vec![ref_with(100, 0xaa, &[(1, 2)])];
        let status = BatchStatus::ProofPending { da: refs.clone() };
        assert_eq!(da_refs_from_status(&status).len(), 1);
    }

    #[test]
    fn da_refs_from_status_returns_empty_for_genesis() {
        let status = BatchStatus::Genesis;
        assert!(da_refs_from_status(&status).is_empty());
    }

    #[test]
    fn build_ledger_refs_is_empty_for_no_da() {
        let refs: Vec<L1DaBlockRef> = Vec::new();
        let lr = build_ledger_refs(&refs, None);
        assert!(lr.l1_header_refs().is_empty());
    }

    #[test]
    fn build_ledger_refs_sorts_ascending_by_height() {
        let refs = vec![
            ref_with(200, 0xbb, &[]),
            ref_with(100, 0xaa, &[]),
            ref_with(150, 0xcc, &[]),
        ];
        // Safe tip lands on the highest DA block — no extra claim
        // appended; the highest-idx entry must still match the tip
        // the proof anchors to.
        let safe_tip = Some(SafeTip {
            height: 200,
            hash: [0xbb; 32],
        });
        let lr = build_ledger_refs(&refs, safe_tip);
        let claims = lr.l1_header_refs();
        assert_eq!(claims.len(), 3);
        assert_eq!(claims[0].idx(), 100);
        assert_eq!(claims[1].idx(), 150);
        assert_eq!(claims[2].idx(), 200);
        assert_eq!(claims[2].entry_hash().as_ref(), &[0xbb; 32]);
    }

    #[test]
    fn build_ledger_refs_appends_safe_tip_above_da() {
        // Safe tip is `l1_reorg_safe_depth` blocks past the DA tip —
        // there should be an extra claim for the deeper anchor, and the
        // highest-idx entry's hash should match `safe_tip.hash` so the
        // in-guest `bind_da_witness_to_ledger_refs` finds the right tip.
        let refs = vec![ref_with(100, 0xaa, &[]), ref_with(150, 0xcc, &[])];
        let safe_tip = Some(SafeTip {
            height: 200,
            hash: [0xee; 32],
        });
        let lr = build_ledger_refs(&refs, safe_tip);
        let claims = lr.l1_header_refs();
        assert_eq!(claims.len(), 3);
        assert_eq!(claims[2].idx(), 200);
        assert_eq!(claims[2].entry_hash().as_ref(), &[0xee; 32]);
    }

    #[test]
    fn build_ledger_refs_skips_safe_tip_when_equal_to_da_tip() {
        // DA freshly published — safe tip cannot sit above the DA tip,
        // so no extra claim. Highest-idx entry stays the DA tip.
        let refs = vec![ref_with(100, 0xaa, &[]), ref_with(150, 0xcc, &[])];
        let safe_tip = Some(SafeTip {
            height: 150,
            hash: [0xcc; 32],
        });
        let lr = build_ledger_refs(&refs, safe_tip);
        let claims = lr.l1_header_refs();
        assert_eq!(claims.len(), 2);
        assert_eq!(claims[1].idx(), 150);
    }

    /// Stub `SafeTipReader` with predictable answers; panics if asked
    /// for a height it wasn't configured for, so accidental RPC use
    /// shows up in the test rather than silently passing.
    struct StubReader {
        chain_tip: u64,
        hash_at_height: HashMap<u64, [u8; 32]>,
    }

    impl SafeTipReader for StubReader {
        async fn chain_tip_height(&self) -> ProverResult<u64> {
            Ok(self.chain_tip)
        }

        async fn block_hash_at(&self, height: u64) -> ProverResult<[u8; 32]> {
            self.hash_at_height.get(&height).copied().ok_or_else(|| {
                PaasError::Storage(format!(
                    "StubReader: hash not configured for height {height}"
                ))
            })
        }
    }

    #[tokio::test]
    async fn select_safe_tip_returns_none_for_no_da() {
        let btc = StubReader {
            chain_tip: 1000,
            hash_at_height: HashMap::new(),
        };
        let tip = select_safe_tip(&[], 6, &btc).await.unwrap();
        assert!(tip.is_none());
    }

    #[tokio::test]
    async fn select_safe_tip_picks_deeper_anchor_when_chain_advanced() {
        let refs = vec![ref_with(100, 0xaa, &[]), ref_with(150, 0xcc, &[])];
        let btc = StubReader {
            chain_tip: 200,
            hash_at_height: HashMap::from([(194, [0x55; 32])]),
        };
        // chain_tip - depth = 200 - 6 = 194 > 150 (max DA height).
        let tip = select_safe_tip(&refs, 6, &btc).await.unwrap().unwrap();
        assert_eq!(tip.height, 194);
        assert_eq!(tip.hash, [0x55; 32]);
    }

    #[tokio::test]
    async fn select_safe_tip_falls_back_to_da_tip_when_da_fresh() {
        let refs = vec![ref_with(100, 0xaa, &[]), ref_with(150, 0xcc, &[])];
        // chain_tip - depth = 152 - 6 = 146 < 150 (max DA height) →
        // anchor at the DA tip; reuse its hash without RPC round-trip
        // (StubReader has no entry at 146, so any RPC would error).
        let btc = StubReader {
            chain_tip: 152,
            hash_at_height: HashMap::new(),
        };
        let tip = select_safe_tip(&refs, 6, &btc).await.unwrap().unwrap();
        assert_eq!(tip.height, 150);
        assert_eq!(tip.hash, [0xcc; 32]);
    }

    #[tokio::test]
    async fn select_safe_tip_zero_depth_picks_chain_tip() {
        let refs = vec![ref_with(100, 0xaa, &[])];
        let btc = StubReader {
            chain_tip: 200,
            hash_at_height: HashMap::from([(200, [0x77; 32])]),
        };
        let tip = select_safe_tip(&refs, 0, &btc).await.unwrap().unwrap();
        assert_eq!(tip.height, 200);
        assert_eq!(tip.hash, [0x77; 32]);
    }
}
