//! Acct (outer/update) [`ProofSpec`].
//!
//! Task = [`BatchId`] (newtype-wrapped). Program = [`EeAcctProgram`];
//! `fetch_input` reads chunk receipts (from the shared paas
//! `ReceiptStore`) and the prior batch's end-state, then assembles
//! `ee_acct_runtime::PrivateInput` + `snark_acct_runtime::PrivateInput`.
//!
//! DA witnesses are stubbed (`LedgerRefs::new_empty()`) per the EE
//! account update doc's "Open Questions" section. See
//! `experimental/evgeniy/ee-da-wiring.md` for the bridge plan.

use std::{fmt, sync::Arc};

use alpen_ee_common::{BatchId, BatchStorage, ExecBlockStorage, Storage};
use alpen_ee_database::EeNodeStorage;
use async_trait::async_trait;
use rsp_primitives::genesis::Genesis;
use ssz::{Decode, Encode as _};
use strata_acct_types::Hash;
use strata_codec::encode_to_vec;
use strata_ee_acct_runtime::{ChunkInput, EePrivateInput};
use strata_ee_acct_types::UpdateExtraData;
use strata_ee_chain_types::ChunkTransition;
use strata_paas::{ProofSpec, ProverError as PaasError, ProverResult, ReceiptStore};
use strata_proofimpl_alpen_acct::{EeAcctProgram, EeAcctProofInput};
use strata_snark_acct_runtime::{Coinput, IInnerState, PrivateInput as UpdatePrivateInput};
use strata_snark_acct_types::{
    LedgerRefs, OutputMessage, OutputTransfer, ProofState, UpdateOutputs, UpdateProofPubParams,
};

use super::ChunkTask;

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

const BATCH_TASK_BYTES: usize = 1 + 64;

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
    genesis: Genesis,
}

impl AcctSpec {
    pub(crate) fn new(
        chunk_receipts: Arc<dyn ReceiptStore>,
        batch_storage: Arc<dyn BatchStorage>,
        storage: Arc<EeNodeStorage>,
        genesis: Genesis,
    ) -> Self {
        Self {
            chunk_receipts,
            batch_storage,
            storage,
            genesis,
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
        let (batch, _status) = self
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
        //    `raw_prev_header` and `raw_partial_pre_state` are carried
        //    through the acct guest but not consumed for verification
        //    today — the guest verifies chunk proofs via predicate key
        //    and checks state transitions via UpdateProofPubParams. The
        //    pre-state fields are reserved for future DA blob consistency
        //    verification inside the acct guest.
        //
        // TODO(STR-1369): once DA verification is added to the acct
        //   guest, source these from the batch's range witness (same
        //   RangeWitnessExtractor used by ChunkSpec).
        let ee_private_input = EePrivateInput::new(Vec::new(), Vec::new(), chunks.clone());

        // 5. Build UpdateProofPubParams from ExecBlockRecords.
        //
        //    Mirrors the submission-side `update_builder.rs` construction:
        //    read each block's ExecBlockRecord to derive processed_inputs,
        //    messages, outputs, next_inbox_msg_idx, and new_tip_blkid.
        //    This is the authoritative source (same data the update
        //    submitter reads), so proof-input and submission agree.
        //
        // TODO(STR-1369): wire real `LedgerRefs` from
        //   `BatchStatus::DaComplete { da }` (see update_builder.rs
        //   lines 139-161 for the reference construction).
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

        let pub_params = UpdateProofPubParams::new(
            cur_state,
            new_state,
            messages,
            LedgerRefs::new_empty(),
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

        Ok(EeAcctProofInput {
            genesis: self.genesis.clone(),
            ee_private_input,
            update_private_input,
        })
    }
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
